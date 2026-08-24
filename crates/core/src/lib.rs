//! Shared zterm domain types and build identity.

pub mod authorization;
pub mod device;
pub mod domain;
pub mod pairing;
pub mod terminal;
pub mod transport;

pub use authorization::{AuthGeneration, AuthorizationSnapshot, AuthorizationStatus};
pub use device::{
    DeviceAlias, DeviceAliasError, DeviceDisplayName, DeviceDisplayNameError, DeviceSummary,
    DeviceSummaryError, MAX_DEVICE_ALIAS_BYTES, MAX_DEVICE_DISPLAY_NAME_BYTES, MAX_RELAY_URL_BYTES,
    RESERVED_DEVICE_ALIAS, RelayHint, RelayHintError, short_endpoint_id,
};
pub use domain::{
    AttachmentId, AttachmentPrincipal, Capabilities, ConnectionAttemptId, ControllerLease,
    DEFAULT_SESSION_NAME, DaemonIncarnation, DeviceId, DeviceIdTextError, DomainErrorKind,
    EphemeralOperationId, IdLengthError, MAX_SESSION_NAME_BYTES, OperationId, OperationLease,
    OperationOutcome, OperationWindow, OperationWindowError, PairNonce, PairOfferId,
    ResourceLimits, ResumeViewId, Revision, SessionEndReason, SessionId, SessionIdTextError,
    SessionName, SessionNameError, SessionSelector,
};
pub use pairing::{
    DEFAULT_PAIR_TTL_SECONDS, MAX_DEVICE_NAME_BYTES, MAX_PAIR_TTL_SECONDS, MIN_PAIR_TTL_SECONDS,
    PAIR_FINGERPRINT_BYTES, PAIR_NONCE_BYTES, PAIR_OFFER_ID_BYTES, PAIR_PROTOCOL_VERSION,
    PAIR_SECRET_BYTES, PAIR_TICKET_FORMAT_VERSION, PairAccepted, PairBegin, PairChallenge,
    PairFingerprint, PairFingerprintError, PairHandshakeBudget, PairHandshakeBudgetError,
    PairProof, PairSecret, PairSecretError, PairTicketError, PairTicketFields, PairTranscript,
    validate_pair_ttl,
};
pub use transport::{
    ConnectionCandidateKey, ConnectionError, ConnectionHello, ConnectionWelcome,
    MAX_PAIR_HANDSHAKE_BYTES, MAX_PAIR_HELLO_FRAME_BYTES, MAX_RELAY_HINTS, MAX_TICKET_TEXT_BYTES,
    TransportLimits, TransportLimitsError, designated_primary,
};

/// Human-readable name of the active implementation phase.
pub const PHASE_NAME: &str = "phase-one-core-local-daemon";

/// Current persistent-state schema version.
pub const STATE_SCHEMA_VERSION: u32 = 1;

/// Immutable identity exposed by build and status projections.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildIdentity {
    /// Cargo package version.
    pub version: &'static str,
    /// Current implementation phase.
    pub phase: &'static str,
}

impl BuildIdentity {
    /// Returns the identity for this workspace build.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            phase: PHASE_NAME,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_identity_matches_workspace_constants() {
        let identity = BuildIdentity::current();
        assert_eq!(identity.phase, PHASE_NAME);
        assert_eq!(identity.version, env!("CARGO_PKG_VERSION"));
    }
}
