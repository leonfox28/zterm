//! Inbound device authorization generation and snapshot values.
//!
//! A host stores one authorization truth per remote [`crate::DeviceId`] in
//! SQLite. The generation is a monotonic `u64` that must remain representable
//! as a signed SQLite integer, so every mutation uses checked arithmetic and
//! refuses to wrap past `i64::MAX`.

use std::fmt;

/// Monotonic inbound authorization generation for one remote device.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AuthGeneration(u64);

impl AuthGeneration {
    /// No authorization has ever been granted.
    pub const ZERO: Self = Self(0);

    /// Ceiling enforced because SQLite stores the generation in a signed 64-bit
    /// column and the state store rejects any value above it.
    pub const SQLITE_MAX: u64 = i64::MAX as u64;

    /// Constructs a generation, rejecting any value above the SQLite signed
    /// 64-bit ceiling so [`Self::to_i64`] can never wrap.
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value > Self::SQLITE_MAX {
            None
        } else {
            Some(Self(value))
        }
    }

    /// Returns the wire/SQLite integer.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next generation, or `None` at the signed 64-bit ceiling.
    #[must_use]
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(next) if next <= Self::SQLITE_MAX => Some(Self(next)),
            _ => None,
        }
    }

    /// Projects the generation into its SQLite signed 64-bit representation.
    ///
    /// Infallible: every constructor is bounded to `<= i64::MAX`, so the cast
    /// cannot wrap.
    #[must_use]
    pub const fn to_i64(self) -> i64 {
        self.0 as i64
    }

    /// Validates a signed 64-bit SQLite generation value.
    #[must_use]
    pub const fn from_i64(value: i64) -> Option<Self> {
        if value < 0 {
            None
        } else {
            Self::new(value as u64)
        }
    }
}

impl fmt::Display for AuthGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Inbound authorization status of one remote device on this host.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AuthorizationStatus {
    /// No authorization row exists for this device.
    #[default]
    None,
    /// The device may currently control this host.
    Authorized,
    /// The device was authorized and later revoked.
    Revoked,
}

/// Immutable point-in-time authorization truth for one remote device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizationSnapshot {
    /// Current inbound status.
    pub status: AuthorizationStatus,
    /// Current inbound authorization generation.
    pub generation: AuthGeneration,
}

impl AuthorizationSnapshot {
    /// Snapshot for a device with no inbound authorization record.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            status: AuthorizationStatus::None,
            generation: AuthGeneration::ZERO,
        }
    }

    /// Returns whether the snapshot admits business requests.
    #[must_use]
    pub const fn is_authorized(self) -> bool {
        matches!(self.status, AuthorizationStatus::Authorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_never_wraps_past_the_sqlite_signed_ceiling() {
        let max = AuthGeneration::new(AuthGeneration::SQLITE_MAX).expect("ceiling is in range");
        assert_eq!(AuthGeneration::ZERO.get(), 0);
        assert_eq!(AuthGeneration::ZERO.checked_next(), AuthGeneration::new(1));
        assert_eq!(AuthGeneration::new(AuthGeneration::SQLITE_MAX), Some(max));
        assert_eq!(AuthGeneration::new(AuthGeneration::SQLITE_MAX + 1), None);
        assert_eq!(AuthGeneration::new(u64::MAX), None);
        assert_eq!(max.checked_next(), None);
        assert_eq!(max.to_i64(), i64::MAX);
    }

    #[test]
    fn generation_round_trips_through_sqlite_i64_without_wrapping() {
        for value in [0, 1, 41, i64::MAX] {
            let generation = AuthGeneration::from_i64(value).expect("non-negative SQLite value");
            assert_eq!(generation.to_i64(), value);
            assert_eq!(
                AuthGeneration::from_i64(generation.to_i64()),
                Some(generation)
            );
        }
        assert_eq!(AuthGeneration::from_i64(-1), None);
        // An unchecked wire u64 above the ceiling is rejected, never wrapped.
        assert_eq!(AuthGeneration::new(i64::MAX as u64 + 1), None);
    }

    #[test]
    fn snapshot_projects_authorization_truth() {
        let none = AuthorizationSnapshot::none();
        assert!(!none.is_authorized());
        assert_eq!(none.status, AuthorizationStatus::None);
        assert_eq!(none.generation, AuthGeneration::ZERO);

        let authorized = AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: AuthGeneration::new(3).expect("3 is in range"),
        };
        assert!(authorized.is_authorized());
    }
}
