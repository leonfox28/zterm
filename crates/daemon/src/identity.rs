//! Persistent Iroh identity with create-without-replace semantics.

use std::fmt;
use std::fs;
use std::io::Write;

use iroh::SecretKey;
use zterm_core::{DeviceId, DomainErrorKind};
use zterm_platform::user_state::{UserPaths, atomic_create, validate_regular_file};

use crate::error::DaemonError;

/// Loaded long-term device identity. Secret bytes are never formatted.
pub struct DeviceIdentity {
    secret_key: SecretKey,
}

impl DeviceIdentity {
    /// Loads exactly 32 raw secret-key bytes from a managed file.
    pub fn load(paths: &UserPaths) -> Result<Self, DaemonError> {
        validate_regular_file(paths.identity(), paths.uid())
            .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
        let bytes = fs::read(paths.identity()).map_err(|error| {
            DaemonError::new(DomainErrorKind::IdentityInvalid, error.to_string())
        })?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
            DaemonError::new(
                DomainErrorKind::IdentityInvalid,
                format!(
                    "identity.key must contain exactly 32 bytes, got {}",
                    bytes.len()
                ),
            )
        })?;
        Ok(Self {
            secret_key: SecretKey::from_bytes(&bytes),
        })
    }

    /// Generates and atomically creates a new identity without replacement.
    pub fn create(paths: &UserPaths) -> Result<Self, DaemonError> {
        let identity = Self {
            secret_key: SecretKey::generate(),
        };
        let bytes = identity.secret_key.to_bytes();
        atomic_create(paths.identity(), paths.uid(), |file| file.write_all(&bytes))
            .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
        Ok(identity)
    }

    /// Public 32-byte device identity.
    #[must_use]
    pub fn device_id(&self) -> DeviceId {
        DeviceId::from_array(*self.secret_key.public().as_bytes())
    }

    /// Iroh's canonical public endpoint encoding.
    #[must_use]
    pub fn endpoint_id(&self) -> String {
        self.secret_key.public().to_string()
    }

    /// Transfers the long-term secret into the daemon-owned network supervisor.
    ///
    /// This is deliberately crate-private: CLI and protocol layers can only
    /// observe [`Self::device_id`] and [`Self::endpoint_id`]. Consuming `self`
    /// also prevents a second subsystem from accidentally retaining an
    /// independent owner of the same long-term secret.
    pub(crate) fn into_secret_key(self) -> SecretKey {
        self.secret_key
    }
}

impl fmt::Debug for DeviceIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceIdentity")
            .field("device_id", &self.device_id())
            .finish_non_exhaustive()
    }
}
