//! Directional device projection and the single owner of local alias
//! reservations shared by pair acceptance and device rename.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use zterm_core::{
    AuthorizationSnapshot, AuthorizationStatus, DeviceAlias, DeviceDisplayName, DeviceId,
    DomainErrorKind,
};

use crate::error::DaemonError;
use crate::store::{RouteCacheDiagnostic, StoreHandle};

/// Merged projection of one device across the outbound address book and the
/// inbound authorization registry.
///
/// Alias, remote name, and route verification describe what this host knows
/// about *reaching* the device; the authorization snapshot describes whether
/// the device may *control* this host. The two directions are merged only for
/// display and never conflated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceProjection {
    /// Remote device public identity.
    pub device_id: DeviceId,
    /// Outbound local alias, when the device is in the address book.
    pub alias: Option<DeviceAlias>,
    /// Outbound remote display name, when known.
    pub remote_name: Option<DeviceDisplayName>,
    /// Whether an outbound relay route was verified.
    pub route_verified: bool,
    /// Non-fatal reason a persisted route cache was ignored.
    pub route_cache_diagnostic: Option<RouteCacheDiagnostic>,
    /// Inbound authorization snapshot.
    pub auth: AuthorizationSnapshot,
    /// Inbound pairing timestamp, when the device was authorized.
    pub paired_at_unix: Option<i64>,
    /// Inbound last authenticated handshake timestamp, when recorded.
    pub last_seen_at_unix: Option<i64>,
}

/// In-memory alias reservation state shared by every accept/rename.
#[derive(Default)]
struct ReservationState {
    aliases: BTreeMap<DeviceAlias, ReservationEntry>,
}

/// One reservation entry, ref-counted so multiple same-device reservations of
/// the same alias release it only when the last guard drops.
struct ReservationEntry {
    device_id: DeviceId,
    count: usize,
}

/// Opaque exact Session target returned by the daemon-side resolver.
///
/// The value contains no alias. Holding it across lease allocation and retry
/// therefore cannot be retargeted by a concurrent alias rename.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResolvedSessionTarget(ResolvedSessionTargetKind);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum ResolvedSessionTargetKind {
    Local,
    Device(DeviceId),
}

impl ResolvedSessionTarget {
    /// Returns whether this exact target is the current local daemon.
    #[must_use]
    pub const fn is_local(self) -> bool {
        matches!(self.0, ResolvedSessionTargetKind::Local)
    }

    /// Returns the frozen full device identity for a remote target.
    #[must_use]
    pub const fn device_id(self) -> Option<DeviceId> {
        match self.0 {
            ResolvedSessionTargetKind::Local => None,
            ResolvedSessionTargetKind::Device(device_id) => Some(device_id),
        }
    }

    pub(crate) const fn local() -> Self {
        Self(ResolvedSessionTargetKind::Local)
    }

    pub(crate) const fn device(device_id: DeviceId) -> Self {
        Self(ResolvedSessionTargetKind::Device(device_id))
    }
}

/// The single owner of the directional device merge and alias reservations.
#[derive(Clone)]
pub struct DeviceDirectory {
    store: StoreHandle,
    reservations: Arc<Mutex<ReservationState>>,
}

impl DeviceDirectory {
    /// Creates a directory backed by the given store handle.
    #[must_use]
    pub fn new(store: StoreHandle) -> Self {
        Self {
            store,
            reservations: Arc::new(Mutex::new(ReservationState::default())),
        }
    }

    /// Resolves one user selector to a frozen exact Session target.
    ///
    /// Alias comparison is exact and case-sensitive. A canonical identifier is
    /// exactly 64 lowercase hexadecimal bytes; ID-looking uppercase, short, or
    /// prefix text is rejected instead of becoming a fuzzy lookup.
    pub fn resolve_session_target(
        &self,
        selector: &str,
        deadline: Instant,
    ) -> Result<ResolvedSessionTarget, DaemonError> {
        if selector == zterm_core::RESERVED_DEVICE_ALIAS {
            return Ok(ResolvedSessionTarget::local());
        }

        let bytes = selector.as_bytes();
        let looks_hex = !bytes.is_empty() && bytes.iter().all(u8::is_ascii_hexdigit);
        if bytes.len() == DeviceId::CANONICAL_TEXT_LENGTH && looks_hex {
            if bytes.iter().any(u8::is_ascii_uppercase) {
                return Err(invalid_target_selector(
                    "device IDs must use the canonical lowercase hexadecimal form",
                ));
            }
            let device_id = selector.parse::<DeviceId>().map_err(|error| {
                invalid_target_selector(format!("invalid canonical device ID: {error}"))
            })?;
            if let Some(alias_owner) = self.store.alias_owner(selector.to_owned(), deadline)?
                && alias_owner != device_id
            {
                return Err(invalid_target_selector(
                    "selector is ambiguous between an exact alias and a canonical device ID",
                ));
            }
            return self.require_outbound_device(device_id, deadline);
        }

        let alias = DeviceAlias::new(selector.to_owned()).map_err(|error| {
            invalid_target_selector(format!("invalid exact device alias: {error}"))
        })?;
        if let Some(device_id) = self
            .store
            .alias_owner(alias.as_str().to_owned(), deadline)?
        {
            return Ok(ResolvedSessionTarget::device(device_id));
        }
        if looks_hex {
            return Err(invalid_target_selector(
                "short and prefix device IDs are not accepted",
            ));
        }
        Err(DaemonError::new(
            DomainErrorKind::DeviceNotFound,
            "no outbound known device has the exact requested alias",
        ))
    }

    /// Rechecks that one frozen full identity remains an outbound target.
    pub fn require_outbound_device(
        &self,
        device_id: DeviceId,
        deadline: Instant,
    ) -> Result<ResolvedSessionTarget, DaemonError> {
        if self.store.known_device(device_id, deadline)?.is_some() {
            return Ok(ResolvedSessionTarget::device(device_id));
        }
        let authorization = self.store.authorization_snapshot(device_id, deadline)?;
        if authorization.status != AuthorizationStatus::None {
            return Err(DaemonError::new(
                DomainErrorKind::OutboundDirectionDenied,
                "device exists only in the inbound authorization direction",
            ));
        }
        Err(DaemonError::new(
            DomainErrorKind::DeviceNotFound,
            "target is not an outbound known device",
        ))
    }

    /// Lists the merged outbound/inbound projection for every device.
    pub fn list(&self, deadline: Instant) -> Result<Vec<DeviceProjection>, DaemonError> {
        let authorizations = self.store.list_authorizations(deadline)?;
        let known = self.store.list_known_devices(deadline)?;
        let mut by_id: BTreeMap<DeviceId, DeviceProjection> = BTreeMap::new();
        for authorization in authorizations {
            let entry = by_id
                .entry(authorization.device_id)
                .or_insert_with(|| DeviceProjection {
                    device_id: authorization.device_id,
                    alias: None,
                    remote_name: None,
                    route_verified: false,
                    route_cache_diagnostic: None,
                    auth: AuthorizationSnapshot::none(),
                    paired_at_unix: None,
                    last_seen_at_unix: None,
                });
            entry.auth = AuthorizationSnapshot {
                status: authorization.status,
                generation: authorization.generation,
            };
            entry.paired_at_unix = Some(authorization.paired_at_unix);
            entry.last_seen_at_unix = authorization.last_seen_at_unix;
        }
        for device in known {
            let entry = by_id
                .entry(device.device_id)
                .or_insert_with(|| DeviceProjection {
                    device_id: device.device_id,
                    alias: None,
                    remote_name: None,
                    route_verified: false,
                    route_cache_diagnostic: None,
                    auth: AuthorizationSnapshot::none(),
                    paired_at_unix: None,
                    last_seen_at_unix: None,
                });
            entry.alias = Some(device.local_alias);
            entry.remote_name = Some(device.remote_name);
            entry.route_verified = device.route_cache.is_some();
            entry.route_cache_diagnostic = device.route_cache_diagnostic;
        }
        Ok(by_id.into_values().collect())
    }

    /// Atomically reserves an alias for one device before any network work.
    ///
    /// An alias durably owned by another device is rejected up front, and a
    /// concurrent reservation of the same alias by a different device fails
    /// with `device_alias_conflict`; the SQLite unique index remains the final
    /// crash-safe owner. The returned guard releases the reservation on drop.
    pub fn reserve_alias(
        &self,
        device_id: DeviceId,
        alias: DeviceAlias,
        deadline: Instant,
    ) -> Result<AliasReservation, DaemonError> {
        if let Some(owner) = self
            .store
            .alias_owner(alias.as_str().to_owned(), deadline)?
            && owner != device_id
        {
            return Err(DaemonError::new(
                DomainErrorKind::DeviceAliasConflict,
                format!(
                    "alias {:?} is already claimed by another device",
                    alias.as_str()
                ),
            ));
        }
        let mut state = reservation_lock(&self.reservations);
        if let Some(entry) = state.aliases.get_mut(&alias) {
            if entry.device_id != device_id {
                return Err(DaemonError::new(
                    DomainErrorKind::DeviceAliasConflict,
                    format!("alias {:?} is already reserved", alias.as_str()),
                ));
            }
            entry.count = entry.count.checked_add(1).ok_or_else(|| {
                DaemonError::new(
                    DomainErrorKind::ResourceExhausted,
                    "alias reservation reference count exhausted",
                )
            })?;
        } else {
            state.aliases.insert(
                alias.clone(),
                ReservationEntry {
                    device_id,
                    count: 1,
                },
            );
        }
        Ok(AliasReservation {
            reservations: Arc::clone(&self.reservations),
            alias,
            device_id,
        })
    }

    /// Chooses and reserves the alias for a pair accept before network work.
    ///
    /// An explicit alias is exact and conflicts are returned. Without one, a
    /// syntactically valid remote display name is preferred; a reserved or
    /// concurrently/durably claimed value falls back to the deterministic
    /// endpoint-suffixed alias from core.
    pub fn reserve_selected_alias(
        &self,
        device_id: DeviceId,
        remote_name: &DeviceDisplayName,
        explicit: Option<DeviceAlias>,
        deadline: Instant,
    ) -> Result<AliasReservation, DaemonError> {
        if let Some(alias) = explicit {
            return self.reserve_alias(device_id, alias, deadline);
        }
        if let Some(preferred) = DeviceAlias::from_remote_name(remote_name.as_str()) {
            match self.reserve_alias(device_id, preferred, deadline) {
                Ok(reservation) => return Ok(reservation),
                Err(error) if error.kind() == DomainErrorKind::DeviceAliasConflict => {}
                Err(error) => return Err(error),
            }
        }
        self.reserve_alias(
            device_id,
            DeviceAlias::disambiguated(remote_name.as_str(), &device_id),
            deadline,
        )
    }

    /// Renames only the outbound alias for one exact device.
    pub fn rename(
        &self,
        device_id: DeviceId,
        alias: DeviceAlias,
        deadline: Instant,
    ) -> Result<(), DaemonError> {
        let _reservation = self.reserve_alias(device_id, alias.clone(), deadline)?;
        self.store.rename_alias(device_id, alias, deadline)
    }

    /// Returns whether an alias is currently unclaimed in the address book.
    pub fn alias_available(&self, alias: &str, deadline: Instant) -> Result<bool, DaemonError> {
        let alias = DeviceAlias::new(alias.to_owned()).map_err(|error| {
            DaemonError::new(DomainErrorKind::InvalidDeviceAlias, error.to_string())
        })?;
        if reservation_lock(&self.reservations)
            .aliases
            .contains_key(&alias)
        {
            return Ok(false);
        }
        self.store
            .alias_available(alias.as_str().to_owned(), deadline)
    }
}

fn invalid_target_selector(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::InvalidTargetSelector, detail)
}

/// RAII guard holding one in-memory alias reservation.
pub struct AliasReservation {
    reservations: Arc<Mutex<ReservationState>>,
    alias: DeviceAlias,
    device_id: DeviceId,
}

impl fmt::Debug for AliasReservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AliasReservation")
            .field("alias", &self.alias)
            .field("device_id", &self.device_id)
            .finish_non_exhaustive()
    }
}

impl AliasReservation {
    /// Alias held by this reservation.
    #[must_use]
    pub fn alias(&self) -> &DeviceAlias {
        &self.alias
    }
}

impl Drop for AliasReservation {
    fn drop(&mut self) {
        let mut state = reservation_lock(&self.reservations);
        if let Some(entry) = state.aliases.get_mut(&self.alias)
            && entry.device_id == self.device_id
        {
            entry.count = entry.count.saturating_sub(1);
            if entry.count == 0 {
                state.aliases.remove(&self.alias);
            }
        }
    }
}

fn reservation_lock(map: &Mutex<ReservationState>) -> MutexGuard<'_, ReservationState> {
    map.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
