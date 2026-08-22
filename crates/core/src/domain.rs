//! Shared product-domain identifiers, capabilities, limits, and replay state.

use std::collections::BTreeMap;
use std::fmt;

/// Reserved name used by the default create-if-missing attach path.
pub const DEFAULT_SESSION_NAME: &str = "main";
/// Maximum encoded UTF-8 bytes in a user-visible session name.
pub const MAX_SESSION_NAME_BYTES: usize = 64;

macro_rules! fixed_id {
    ($name:ident, $length:expr, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; $length]);

        impl $name {
            /// Number of bytes in the canonical representation.
            pub const LENGTH: usize = $length;

            /// Constructs an identifier from its canonical bytes.
            #[must_use]
            pub const fn from_array(bytes: [u8; $length]) -> Self {
                Self(bytes)
            }

            /// Validates and copies the canonical byte representation.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdLengthError> {
                let actual = bytes.len();
                let bytes = bytes.try_into().map_err(|_| IdLengthError {
                    identifier: stringify!($name),
                    expected: $length,
                    actual,
                })?;
                Ok(Self(bytes))
            }

            /// Returns the canonical byte representation.
            #[must_use]
            pub const fn to_bytes(self) -> [u8; $length] {
                self.0
            }

            /// Borrows the canonical byte representation.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; $length] {
                &self.0
            }
        }
    };
}

fixed_id!(DeviceId, 32, "Stable public identity of one zterm device.");
fixed_id!(
    DaemonIncarnation,
    16,
    "Random identity of one running daemon incarnation."
);
fixed_id!(
    SessionId,
    16,
    "Stable identifier of one live terminal session."
);

/// Validated, case-sensitive name of one live terminal session.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SessionName(String);

impl SessionName {
    /// Validates a user-visible session name without trimming or normalizing it.
    pub fn new(value: impl Into<String>) -> Result<Self, SessionNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(SessionNameError::Empty);
        }
        if value.len() > MAX_SESSION_NAME_BYTES {
            return Err(SessionNameError::TooLong {
                actual: value.len(),
                maximum: MAX_SESSION_NAME_BYTES,
            });
        }
        if value.trim() != value {
            return Err(SessionNameError::SurroundingWhitespace);
        }
        if value.chars().any(char::is_control) {
            return Err(SessionNameError::ControlCharacter);
        }
        Ok(Self(value))
    }

    /// Returns the reserved default session name.
    #[must_use]
    pub fn main() -> Self {
        Self(DEFAULT_SESSION_NAME.to_owned())
    }

    /// Returns whether this is the reserved default name.
    #[must_use]
    pub fn is_main(&self) -> bool {
        self.0 == DEFAULT_SESSION_NAME
    }

    /// Borrows the validated name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Failure while validating a session name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionNameError {
    /// A name must contain at least one byte.
    Empty,
    /// A name exceeded the fixed UTF-8 byte bound.
    TooLong {
        /// Observed byte count.
        actual: usize,
        /// Maximum accepted byte count.
        maximum: usize,
    },
    /// Leading or trailing Unicode whitespace is forbidden.
    SurroundingWhitespace,
    /// Unicode control characters are forbidden.
    ControlCharacter,
}

impl fmt::Display for SessionNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "session name must not be empty"),
            Self::TooLong { actual, maximum } => write!(
                formatter,
                "session name must contain at most {maximum} UTF-8 bytes, got {actual}"
            ),
            Self::SurroundingWhitespace => {
                write!(
                    formatter,
                    "session name must not have surrounding whitespace"
                )
            }
            Self::ControlCharacter => {
                write!(
                    formatter,
                    "session name must not contain control characters"
                )
            }
        }
    }
}

impl std::error::Error for SessionNameError {}

/// Stable way to select one live session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionSelector {
    /// Select by immutable daemon-lifetime identifier.
    Id(SessionId),
    /// Select by current unique name.
    Name(SessionName),
}

/// Why a live session stopped owning its root shell and PTY.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionEndReason {
    /// The root shell exited without an explicit zterm close.
    NaturalExit {
        /// Portable exit code projection.
        exit_code: u32,
        /// Bounded platform signal description, when available.
        signal: Option<String>,
    },
    /// The user explicitly closed this session.
    ExplicitClose,
    /// The per-user daemon is shutting down.
    DaemonStop,
    /// The retained terminal driver failed.
    DriverFailure,
}
fixed_id!(
    AttachmentId,
    16,
    "Identifier of one local or remote view attached to a session."
);

/// Error returned when a fixed-width identifier has the wrong byte length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdLengthError {
    identifier: &'static str,
    expected: usize,
    actual: usize,
}

impl IdLengthError {
    /// Expected byte count.
    #[must_use]
    pub const fn expected(self) -> usize {
        self.expected
    }

    /// Observed byte count.
    #[must_use]
    pub const fn actual(self) -> usize {
        self.actual
    }
}

impl fmt::Display for IdLengthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} must contain exactly {} bytes, got {}",
            self.identifier, self.expected, self.actual
        )
    }
}

impl std::error::Error for IdLengthError {}

/// Monotonic revision of host-authoritative terminal state.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Revision(u64);

impl Revision {
    /// Initial revision before the first mutation.
    pub const ZERO: Self = Self(0);

    /// Constructs a revision from its wire integer.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the wire integer.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next revision, or `None` at the numeric ceiling.
    #[must_use]
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Trust source and identity of a terminal attachment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentPrincipal {
    /// A paired remote endpoint authenticated at the network boundary.
    RemoteEndpoint {
        /// Remote device public identity.
        device_id: DeviceId,
        /// Authorization generation accepted for this connection.
        auth_generation: u64,
    },
    /// A process admitted by the local same-UID peer credential gate.
    LocalSameUid {
        /// Public identity of this daemon.
        own_device_id: DeviceId,
        /// Identifier distinguishing concurrent local views.
        local_view_id: AttachmentId,
    },
}

/// Current controller ownership token for one live session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControllerLease {
    /// Attachment which owns controller input.
    pub attachment_id: AttachmentId,
    /// Monotonic generation changed by each transfer.
    pub generation: u64,
}

/// Negotiated optional product capabilities.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities(u64);

impl Capabilities {
    /// Local daemon lifecycle RPCs.
    pub const LOCAL_LIFECYCLE: u64 = 1 << 0;
    /// Session management RPCs.
    pub const SESSION_SERVICE: u64 = 1 << 1;
    /// Terminal attach/snapshot/delta RPCs.
    pub const TERMINAL_SERVICE: u64 = 1 << 2;
    /// Future device status notifications.
    pub const DEVICE_EVENTS: u64 = 1 << 16;
    /// Future paged terminal history.
    pub const HISTORY_PAGING: u64 = 1 << 17;
    /// Future dedicated Agent status and notification events.
    pub const AGENT_EVENTS: u64 = 1 << 18;

    /// Constructs a capability set while retaining unknown bits.
    #[must_use]
    pub const fn from_bits_retain(bits: u64) -> Self {
        Self(bits)
    }

    /// Returns all known and unknown capability bits.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Returns whether every requested bit is present.
    #[must_use]
    pub const fn contains(self, bits: u64) -> bool {
        self.0 & bits == bits
    }
}

/// Foundation-approved default resource policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceLimits {
    /// Maximum simultaneously live sessions for one user.
    pub max_live_sessions: usize,
    /// Recent terminal history rows retained per session.
    pub recent_history_rows: usize,
    /// Rows used when no controller supplies a viewport.
    pub no_controller_rows: u16,
    /// Columns used when no controller supplies a viewport.
    pub no_controller_columns: u16,
    /// Maximum accepted viewport rows.
    pub max_viewport_rows: u16,
    /// Maximum accepted viewport columns.
    pub max_viewport_columns: u16,
    /// Admission ceiling for summed fixed-cell projections.
    pub aggregate_cell_projection_bytes: usize,
    /// Maximum simultaneous local unary or attachment IPC connections.
    pub max_local_connections: usize,
    /// Maximum accepted relative local request deadline.
    pub max_local_deadline_seconds: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_live_sessions: 8,
            recent_history_rows: 2_000,
            no_controller_rows: 40,
            no_controller_columns: 120,
            max_viewport_rows: 80,
            max_viewport_columns: 240,
            aggregate_cell_projection_bytes: 128 * 1024 * 1024,
            max_local_connections: 32,
            max_local_deadline_seconds: 30,
        }
    }
}

/// Whole-process memory target measured by integration gates, not admission.
pub const DAEMON_RSS_MEASUREMENT_TARGET_BYTES: usize = 256 * 1024 * 1024;

/// Daemon-issued bounded replay lease for one authenticated principal.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationLease {
    /// Random daemon-lifetime identity. Leases never survive daemon restart.
    pub daemon_incarnation: DaemonIncarnation,
    /// Daemon-monotonic issued ordinal for this stable principal/auth generation.
    pub ordinal: u64,
}

/// Stable identifier of one state-changing client operation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationId {
    /// Lease allocated by the daemon before the first mutation.
    pub lease: OperationLease,
    /// Monotonic non-zero sequence within the lease.
    pub sequence: u64,
}

/// Result of running or replaying an operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationOutcome<R> {
    /// The closure ran once and this result was retained.
    Executed(R),
    /// The exact retained result was returned without running the closure.
    Replayed(R),
}

/// Error at the bounded operation replay boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationWindowError {
    /// A replay window must retain at least one result.
    InvalidCapacity,
    /// The operation belongs to another lease or its result was evicted.
    OutcomeUnknown,
}

impl fmt::Display for OperationWindowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCapacity => {
                write!(formatter, "operation replay capacity must be non-zero")
            }
            Self::OutcomeUnknown => write!(formatter, "operation outcome is no longer known"),
        }
    }
}

impl std::error::Error for OperationWindowError {}

/// Bounded exact-result replay for one fixed daemon-issued operation lease.
pub struct OperationWindow<R> {
    lease: OperationLease,
    capacity: usize,
    low_water: u64,
    results: BTreeMap<u64, R>,
}

impl<R: Clone> OperationWindow<R> {
    /// Creates one fixed-lease replay window.
    pub fn new(lease: OperationLease, capacity: usize) -> Result<Self, OperationWindowError> {
        if capacity == 0 {
            return Err(OperationWindowError::InvalidCapacity);
        }
        Ok(Self {
            lease,
            capacity,
            low_water: 0,
            results: BTreeMap::new(),
        })
    }

    /// Runs a new operation exactly once or returns its exact retained result.
    pub fn execute(
        &mut self,
        id: OperationId,
        operation: impl FnOnce() -> R,
    ) -> Result<OperationOutcome<R>, OperationWindowError> {
        if id.lease != self.lease || id.sequence == 0 {
            return Err(OperationWindowError::OutcomeUnknown);
        }
        if let Some(result) = self.results.get(&id.sequence) {
            return Ok(OperationOutcome::Replayed(result.clone()));
        }
        if id.sequence < self.low_water {
            return Err(OperationWindowError::OutcomeUnknown);
        }

        let result = operation();
        self.results.insert(id.sequence, result.clone());
        while self.results.len() > self.capacity {
            let Some((evicted, _)) = self.results.pop_first() else {
                break;
            };
            self.low_water = self.low_water.max(evicted.saturating_add(1));
        }
        Ok(OperationOutcome::Executed(result))
    }

    /// Lowest sequence whose missing result may still be executed.
    #[must_use]
    pub const fn low_water(&self) -> u64 {
        self.low_water
    }
}

/// Stable high-level failure categories shared by services and clients.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainErrorKind {
    /// Persistent identity/configuration has not been initialized.
    NotSetup,
    /// Requested setup differs from committed setup.
    AlreadyConfiguredConflict,
    /// A managed filesystem node failed the trust boundary.
    PathUnsafe,
    /// A managed node has unexpected ownership or permissions.
    PermissionMismatch,
    /// The requested OS integration is not implemented on this platform.
    UnsupportedPlatform,
    /// Identity bytes are malformed.
    IdentityInvalid,
    /// Persistent metadata does not match the identity key.
    IdentityStateMismatch,
    /// Configuration syntax is invalid.
    ConfigSyntax,
    /// Configuration schema is unsupported.
    ConfigVersion,
    /// Infrastructure profile is invalid.
    ConfigProfile,
    /// Persistent schema is newer than this binary.
    SchemaTooNew,
    /// A schema migration failed.
    MigrationFailed,
    /// State storage is unavailable.
    StoreUnavailable,
    /// No daemon is listening.
    DaemonStopped,
    /// Another daemon already owns the instance.
    DaemonAlreadyRunning,
    /// A launched daemon did not become ready in time.
    DaemonStartTimeout,
    /// A local process had the wrong OS user identity.
    PeerUidMismatch,
    /// A request exceeded its deadline.
    DeadlineExceeded,
    /// A request was cancelled before commit.
    Cancelled,
    /// Wire major versions are incompatible.
    WireMajorMismatch,
    /// Numeric message kind is not recognized.
    UnknownKind,
    /// A wire frame exceeds the protocol bound.
    FrameTooLarge,
    /// A control payload exceeds the protocol bound.
    ControlPayloadTooLarge,
    /// Framing or protobuf bytes are malformed.
    MalformedFrame,
    /// A replay result was evicted or belongs to another lease.
    OperationOutcomeUnknown,
    /// A session name failed its product validation contract.
    InvalidSessionName,
    /// A requested session working directory is invalid or inaccessible.
    InvalidWorkingDirectory,
    /// A normal create or rename attempted to use the reserved `main` name.
    ReservedSessionName,
    /// A live session already uses the requested name.
    SessionAlreadyExists,
    /// The selected live session does not exist.
    SessionNotFound,
    /// The selected session already has a controller.
    SessionOccupied,
    /// An attachment has not confirmed the latest full snapshot.
    NotSynchronized,
    /// An attachment no longer owns the controller lease.
    LeaseLost,
    /// A fixed session, viewport, or projection bound would be exceeded.
    ResourceExhausted,
    /// A defined future service is not implemented in this milestone.
    ServiceNotImplemented,
}

impl DomainErrorKind {
    /// Stable snake-case code used by wire and JSON projections.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotSetup => "not_setup",
            Self::AlreadyConfiguredConflict => "already_configured_conflict",
            Self::PathUnsafe => "path_unsafe",
            Self::PermissionMismatch => "permission_mismatch",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::IdentityInvalid => "identity_invalid",
            Self::IdentityStateMismatch => "identity_state_mismatch",
            Self::ConfigSyntax => "config_syntax",
            Self::ConfigVersion => "config_version",
            Self::ConfigProfile => "config_profile",
            Self::SchemaTooNew => "schema_too_new",
            Self::MigrationFailed => "migration_failed",
            Self::StoreUnavailable => "store_unavailable",
            Self::DaemonStopped => "daemon_stopped",
            Self::DaemonAlreadyRunning => "daemon_already_running",
            Self::DaemonStartTimeout => "daemon_start_timeout",
            Self::PeerUidMismatch => "peer_uid_mismatch",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
            Self::WireMajorMismatch => "wire_major_mismatch",
            Self::UnknownKind => "unknown_kind",
            Self::FrameTooLarge => "frame_too_large",
            Self::ControlPayloadTooLarge => "control_payload_too_large",
            Self::MalformedFrame => "malformed_frame",
            Self::OperationOutcomeUnknown => "operation_outcome_unknown",
            Self::InvalidSessionName => "invalid_session_name",
            Self::InvalidWorkingDirectory => "invalid_working_directory",
            Self::ReservedSessionName => "reserved_session_name",
            Self::SessionAlreadyExists => "session_already_exists",
            Self::SessionNotFound => "session_not_found",
            Self::SessionOccupied => "session_occupied",
            Self::NotSynchronized => "not_synchronized",
            Self::LeaseLost => "lease_lost",
            Self::ResourceExhausted => "resource_exhausted",
            Self::ServiceNotImplemented => "service_not_implemented",
        }
    }

    /// Parses the stable snake-case code used by wire and JSON projections.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Some(match code {
            "not_setup" => Self::NotSetup,
            "already_configured_conflict" => Self::AlreadyConfiguredConflict,
            "path_unsafe" => Self::PathUnsafe,
            "permission_mismatch" => Self::PermissionMismatch,
            "unsupported_platform" => Self::UnsupportedPlatform,
            "identity_invalid" => Self::IdentityInvalid,
            "identity_state_mismatch" => Self::IdentityStateMismatch,
            "config_syntax" => Self::ConfigSyntax,
            "config_version" => Self::ConfigVersion,
            "config_profile" => Self::ConfigProfile,
            "schema_too_new" => Self::SchemaTooNew,
            "migration_failed" => Self::MigrationFailed,
            "store_unavailable" => Self::StoreUnavailable,
            "daemon_stopped" => Self::DaemonStopped,
            "daemon_already_running" => Self::DaemonAlreadyRunning,
            "daemon_start_timeout" => Self::DaemonStartTimeout,
            "peer_uid_mismatch" => Self::PeerUidMismatch,
            "deadline_exceeded" => Self::DeadlineExceeded,
            "cancelled" => Self::Cancelled,
            "wire_major_mismatch" => Self::WireMajorMismatch,
            "unknown_kind" => Self::UnknownKind,
            "frame_too_large" => Self::FrameTooLarge,
            "control_payload_too_large" => Self::ControlPayloadTooLarge,
            "malformed_frame" => Self::MalformedFrame,
            "operation_outcome_unknown" => Self::OperationOutcomeUnknown,
            "invalid_session_name" => Self::InvalidSessionName,
            "invalid_working_directory" => Self::InvalidWorkingDirectory,
            "reserved_session_name" => Self::ReservedSessionName,
            "session_already_exists" => Self::SessionAlreadyExists,
            "session_not_found" => Self::SessionNotFound,
            "session_occupied" => Self::SessionOccupied,
            "not_synchronized" => Self::NotSynchronized,
            "lease_lost" => Self::LeaseLost,
            "resource_exhausted" => Self::ResourceExhausted,
            "service_not_implemented" => Self::ServiceNotImplemented,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_principals_capabilities_and_limits_preserve_contracts() {
        assert_eq!(
            DeviceId::from_bytes(&[7; 31])
                .err()
                .map(IdLengthError::actual),
            Some(31)
        );
        let device = DeviceId::from_array([7; 32]);
        let view = AttachmentId::from_array([9; 16]);
        assert_ne!(
            AttachmentPrincipal::LocalSameUid {
                own_device_id: device,
                local_view_id: view,
            },
            AttachmentPrincipal::RemoteEndpoint {
                device_id: device,
                auth_generation: 1,
            }
        );

        let unknown = 1_u64 << 63;
        let capabilities = Capabilities::from_bits_retain(Capabilities::LOCAL_LIFECYCLE | unknown);
        assert!(capabilities.contains(Capabilities::LOCAL_LIFECYCLE));
        assert_eq!(capabilities.bits() & unknown, unknown);

        let limits = ResourceLimits::default();
        assert_eq!(limits.max_live_sessions, 8);
        assert_eq!(limits.recent_history_rows, 2_000);
        assert_eq!(limits.aggregate_cell_projection_bytes, 128 * 1024 * 1024);
        assert_eq!(DAEMON_RSS_MEASUREMENT_TARGET_BYTES, 256 * 1024 * 1024);
        assert_eq!(
            DomainErrorKind::from_code(DomainErrorKind::DaemonStopped.code()),
            Some(DomainErrorKind::DaemonStopped)
        );
        assert_eq!(DomainErrorKind::from_code("future_error"), None);
    }

    #[test]
    fn session_names_and_selectors_have_one_validation_owner() {
        let main = SessionName::main();
        assert!(main.is_main());
        assert_eq!(main.as_str(), DEFAULT_SESSION_NAME);
        assert_eq!(SessionName::new(""), Err(SessionNameError::Empty));
        assert_eq!(
            SessionName::new(" padded"),
            Err(SessionNameError::SurroundingWhitespace)
        );
        assert_eq!(
            SessionName::new("line\nbreak"),
            Err(SessionNameError::ControlCharacter)
        );
        assert!(matches!(
            SessionName::new("界".repeat(22)),
            Err(SessionNameError::TooLong {
                actual: 66,
                maximum: 64
            })
        ));
        let named = SessionName::new("Work-界").expect("bounded Unicode name");
        assert_eq!(named.as_str(), "Work-界");
        assert_ne!(named, SessionName::new("work-界").expect("case-sensitive"));
        assert_eq!(
            SessionSelector::Name(named.clone()),
            SessionSelector::Name(named)
        );

        for kind in [
            DomainErrorKind::InvalidSessionName,
            DomainErrorKind::InvalidWorkingDirectory,
            DomainErrorKind::ReservedSessionName,
            DomainErrorKind::SessionAlreadyExists,
            DomainErrorKind::SessionNotFound,
            DomainErrorKind::SessionOccupied,
            DomainErrorKind::NotSynchronized,
            DomainErrorKind::LeaseLost,
            DomainErrorKind::ResourceExhausted,
        ] {
            assert_eq!(DomainErrorKind::from_code(kind.code()), Some(kind));
        }
    }

    #[test]
    fn operation_window_replays_exact_results_and_never_reruns_evicted_ids() {
        let lease = OperationLease {
            daemon_incarnation: DaemonIncarnation::from_array([4; 16]),
            ordinal: 41,
        };
        let mut window = OperationWindow::new(lease, 2).expect("non-zero replay capacity");
        let mut executions = 0;
        let first = OperationId { lease, sequence: 4 };
        assert_eq!(
            window.execute(first, || {
                executions += 1;
                Result::<_, &'static str>::Err("retained failure")
            }),
            Ok(OperationOutcome::Executed(Err("retained failure")))
        );
        assert_eq!(
            window.execute(first, || {
                executions += 1;
                Ok(99)
            }),
            Ok(OperationOutcome::Replayed(Err("retained failure")))
        );
        assert_eq!(executions, 1);

        let two = OperationId { lease, sequence: 5 };
        let three = OperationId { lease, sequence: 6 };
        assert!(matches!(
            window.execute(two, || Ok(2)),
            Ok(OperationOutcome::Executed(Ok(2)))
        ));
        assert!(matches!(
            window.execute(three, || Ok(3)),
            Ok(OperationOutcome::Executed(Ok(3)))
        ));
        assert_eq!(window.low_water(), 5);
        assert_eq!(
            window.execute(first, || Ok(100)),
            Err(OperationWindowError::OutcomeUnknown)
        );
        assert_eq!(
            window.execute(
                OperationId {
                    lease: OperationLease {
                        ordinal: 99,
                        ..lease
                    },
                    sequence: 7
                },
                || Ok(7)
            ),
            Err(OperationWindowError::OutcomeUnknown)
        );

        let second_lease = OperationLease {
            ordinal: 7,
            ..lease
        };
        let mut out_of_order = OperationWindow::new(second_lease, 2).expect("bounded window");
        assert!(matches!(
            out_of_order.execute(
                OperationId {
                    lease: second_lease,
                    sequence: 10,
                },
                || "ten"
            ),
            Ok(OperationOutcome::Executed("ten"))
        ));
        assert!(matches!(
            out_of_order.execute(
                OperationId {
                    lease: second_lease,
                    sequence: 12,
                },
                || "twelve"
            ),
            Ok(OperationOutcome::Executed("twelve"))
        ));
        assert!(matches!(
            out_of_order.execute(
                OperationId {
                    lease: second_lease,
                    sequence: 11,
                },
                || "eleven"
            ),
            Ok(OperationOutcome::Executed("eleven"))
        ));
        assert_eq!(out_of_order.low_water(), 11);
        assert_eq!(
            out_of_order.execute(
                OperationId {
                    lease: second_lease,
                    sequence: 10,
                },
                || "must-not-run"
            ),
            Err(OperationWindowError::OutcomeUnknown)
        );
    }
}
