//! Versioned protobuf DTOs and the one bounded zterm frame codec.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use prost::Message;
use zeroize::{Zeroize, Zeroizing};
use zterm_core::terminal::{
    ActiveScreen, TerminalDelta, TerminalModes, TerminalMouseEncoding, TerminalMouseMode,
    TerminalSize, TerminalSnapshot,
};
use zterm_core::{
    AttachmentId, AuthGeneration, AuthorizationStatus, Capabilities, ConnectionAttemptId,
    ConnectionError, ConnectionHello, ConnectionWelcome, DaemonIncarnation, DeviceAlias,
    DeviceAliasError, DeviceId, DeviceSummary, DeviceSummaryError, EphemeralOperationId,
    IdLengthError, MAX_TICKET_TEXT_BYTES, OperationId, OperationLease, PairAccepted, PairBegin,
    PairChallenge, PairFingerprint, PairFingerprintError, PairNonce, PairOfferId, PairProof,
    PairSecret, PairSecretError, PairTicketError, PairTicketFields, RelayHint, RelayHintError,
    ResumeViewId, SessionId,
};

/// Generated version-one protocol DTOs.
pub mod v1 {
    #![allow(missing_docs)]
    include!(concat!(env!("OUT_DIR"), "/zterm.v1.rs"));
}

impl fmt::Debug for v1::WireFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WireFrame")
            .field("wire_major", &self.wire_major)
            .field("kind", &self.kind)
            .field("payload", &"[REDACTED]")
            .field("payload_len", &self.payload.len())
            .field("request_id", &self.request_id)
            .field("deadline_ms", &self.deadline_ms)
            .finish()
    }
}

impl fmt::Debug for v1::PairTicketV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairTicketV1")
            .field("format_version", &self.format_version)
            .field("host_device_id", &self.host_device_id)
            .field("host_name", &self.host_name)
            .field("relay_url_count", &self.relay_urls.len())
            .field("offer_id", &"[REDACTED]")
            .field("secret", &"[REDACTED]")
            .field("expires_at_unix", &self.expires_at_unix)
            .finish()
    }
}

impl fmt::Debug for v1::PairBegin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairBegin")
            .field("offer_id", &"[REDACTED]")
            .field("controller_name", &self.controller_name)
            .field("controller_nonce", &"[REDACTED]")
            .field("pair_protocol_version", &self.pair_protocol_version)
            .finish()
    }
}

impl fmt::Debug for v1::PairChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairChallenge")
            .field("host_nonce", &"[REDACTED]")
            .field("selected_version", &self.selected_version)
            .field("ticket_expiry_unix", &self.ticket_expiry_unix)
            .finish()
    }
}

impl fmt::Debug for v1::PairProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairProof")
            .field("controller_proof", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for v1::PairAccepted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairAccepted")
            .field("authorization_generation", &self.authorization_generation)
            .field("host_confirmation_proof", &"[REDACTED]")
            .field("host_diagnostic_version", &self.host_diagnostic_version)
            .finish()
    }
}

impl fmt::Debug for v1::LocalPairCreateResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalPairCreateResponse")
            .field("ticket", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for v1::LocalPairAcceptRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalPairAcceptRequest")
            .field("ephemeral_operation_id", &"[REDACTED]")
            .field("fingerprint", &"[REDACTED]")
            .field("ticket", &"[REDACTED]")
            .field("alias", &self.alias)
            .finish()
    }
}

impl fmt::Debug for v1::LocalSessionUnaryRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalSessionUnaryRequest")
            .field("target_device_id", &self.target_device_id)
            .field("frame", &"[REDACTED]")
            .field("frame_len", &self.frame.len())
            .finish()
    }
}

impl fmt::Debug for v1::LocalStatusResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalStatusResponse")
            .field("protocol", &self.protocol)
            .field("version", &self.version)
            .field("phase", &self.phase)
            .field("device_id", &self.device_id)
            .field("endpoint_id", &self.endpoint_id)
            .field("device_name", &self.device_name)
            .field("infrastructure_profile", &self.infrastructure_profile)
            .field("started_at_unix", &self.started_at_unix)
            .field("active_session_count", &self.active_session_count)
            .field("active_session_names", &self.active_session_names)
            .field("network_state", &self.network_state)
            .field("endpoint_bound", &self.endpoint_bound)
            .field("network_bind_attempts", &self.network_bind_attempts)
            .field("home_relay", &"[REDACTED]")
            .field("home_relay_present", &!self.home_relay.is_empty())
            .field("address_publish_state", &self.address_publish_state)
            .field("address_lookup_state", &self.address_lookup_state)
            .field(
                "authenticated_connection_count",
                &self.authenticated_connection_count,
            )
            .field("primary_connection_count", &self.primary_connection_count)
            .field("active_stream_count", &self.active_stream_count)
            .field("direct_path_count", &self.direct_path_count)
            .field("relay_path_count", &self.relay_path_count)
            .field("network_diagnostic", &self.network_diagnostic)
            .finish()
    }
}

impl fmt::Debug for v1::LocalValidateSetupRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalValidateSetupRequest")
            .field("device_name", &self.device_name)
            .field("infrastructure_profile", &self.infrastructure_profile)
            .field("relay_url", &"[REDACTED]")
            .field("relay_url_present", &!self.relay_url.is_empty())
            .finish()
    }
}

impl fmt::Debug for v1::RelayRouteCacheV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayRouteCacheV1")
            .field("format_version", &self.format_version)
            .field("relay_url_count", &self.relay_urls.len())
            .finish()
    }
}

impl fmt::Debug for v1::SessionSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionSummary")
            .field("session_id", &self.session_id)
            .field("name", &self.name)
            .field("revision", &self.revision)
            .field("has_controller", &self.has_controller)
            .field("working_directory", &"[REDACTED]")
            .field("working_directory_len", &self.working_directory.len())
            .field("viewport", &self.viewport)
            .finish()
    }
}

impl fmt::Debug for v1::SessionCreateRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionCreateRequest")
            .field("operation_id", &self.operation_id)
            .field("target", &self.target)
            .field("name", &self.name)
            .field("working_directory", &"[REDACTED]")
            .field("working_directory_len", &self.working_directory.len())
            .field("viewport", &self.viewport)
            .finish()
    }
}

impl fmt::Debug for v1::ResumeViewId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResumeViewId([REDACTED])")
    }
}

impl fmt::Debug for v1::TerminalAttachRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalAttachRequest")
            .field("target", &self.target)
            .field("session_id", &self.session_id)
            .field("takeover", &self.takeover)
            .field("session_name", &self.session_name)
            .field("create_main", &self.create_main)
            .field("viewport", &self.viewport)
            .field(
                "resume_view_id",
                &self.resume_view_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field("known_revision", &self.known_revision)
            .finish()
    }
}

impl fmt::Debug for v1::TerminalSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSnapshot")
            .field("session_id", &self.session_id)
            .field("attachment_id", &self.attachment_id)
            .field("revision", &self.revision)
            .field("rows", &self.rows)
            .field("columns", &self.columns)
            .field("screen_ansi", &"[REDACTED]")
            .field("screen_ansi_len", &self.screen_ansi.len())
            .field("recent_history_ansi", &"[REDACTED]")
            .field("recent_history_ansi_len", &self.recent_history_ansi.len())
            .field("active_screen", &self.active_screen)
            .field("modes", &self.modes)
            .finish()
    }
}

impl fmt::Debug for v1::TerminalDelta {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalDelta")
            .field("from_revision", &self.from_revision)
            .field("to_revision", &self.to_revision)
            .field("ansi", &"[REDACTED]")
            .field("ansi_len", &self.ansi.len())
            .field("rows", &self.rows)
            .field("columns", &self.columns)
            .field("active_screen", &self.active_screen)
            .field("modes", &self.modes)
            .field("attachment_id", &self.attachment_id)
            .finish()
    }
}

impl fmt::Debug for v1::TerminalInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalInput")
            .field("operation_id", &self.operation_id)
            .field("attachment_id", &self.attachment_id)
            .field("bytes", &"[REDACTED]")
            .field("input_len", &self.bytes.len())
            .finish()
    }
}

/// Product wire major shared by local IPC, `zterm/1`, and `zterm-pair/1`.
pub const WIRE_MAJOR: u32 = 1;
/// Current persistent-state schema exposed in readiness/status.
pub const STATE_SCHEMA_VERSION: u32 = zterm_core::STATE_SCHEMA_VERSION;
/// Maximum encoded `WireFrame` body size.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
/// Maximum concrete control-message payload size.
pub const MAX_CONTROL_PAYLOAD_BYTES: usize = 1024 * 1024;
/// Maximum bytes in an unsigned 64-bit varint prefix.
pub const MAX_VARINT_BYTES: usize = 10;
const TERMINAL_SNAPSHOT_FRAME_HEADROOM: usize = 4 * 1024;

/// Stable validated wire message kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum WireKind {
    /// Local daemon readiness request.
    LocalReadinessRequest = 1,
    /// Local daemon readiness response.
    LocalReadinessResponse = 2,
    /// Local daemon status request.
    LocalStatusRequest = 3,
    /// Local daemon status response.
    LocalStatusResponse = 4,
    /// Validate existing setup request.
    LocalValidateSetupRequest = 5,
    /// Validate existing setup response.
    LocalValidateSetupResponse = 6,
    /// Graceful daemon stop request.
    LocalStopRequest = 7,
    /// Graceful daemon stop response.
    LocalStopResponse = 8,
    /// Manual update preflight request.
    LocalUpdatePreflightRequest = 9,
    /// Manual update preflight response.
    LocalUpdatePreflightResponse = 10,
    /// Typed service error response.
    ServiceErrorResponse = 11,
    /// Local one-time pairing ticket creation request.
    LocalPairCreateRequest = 12,
    /// Local one-time pairing ticket creation response.
    LocalPairCreateResponse = 13,
    /// Local one-time pairing acceptance request.
    LocalPairAcceptRequest = 14,
    /// Local one-time pairing acceptance response.
    LocalPairAcceptResponse = 15,
    /// Local device list request.
    LocalDeviceListRequest = 16,
    /// Local device list response.
    LocalDeviceListResponse = 17,
    /// Local device alias rename request.
    LocalDeviceRenameRequest = 18,
    /// Local device alias rename response.
    LocalDeviceRenameResponse = 19,
    /// Local inbound authorization revoke request.
    LocalDeviceRevokeRequest = 20,
    /// Local inbound authorization revoke response.
    LocalDeviceRevokeResponse = 21,
    /// Resolve one exact local daemon target selector.
    LocalTargetResolveRequest = 22,
    /// Frozen local/device target selected by the daemon.
    LocalTargetResolveResponse = 23,
    /// Same-UID envelope containing one preencoded remote Session unary.
    LocalSessionUnaryRequest = 24,
    /// Controller opens a pairing handshake.
    PairBegin = 100,
    /// Host responds to a pairing handshake.
    PairChallenge = 101,
    /// Controller submits its secret-possession proof.
    PairProof = 102,
    /// Host confirms a committed authorization.
    PairAccepted = 103,
    /// First authenticated frame on a normal connection.
    ConnectionHello = 104,
    /// Responder half of a normal connection handshake.
    ConnectionWelcome = 105,
    /// Session list request.
    SessionListRequest = 200,
    /// Session list response.
    SessionListResponse = 201,
    /// Session creation request.
    SessionCreateRequest = 202,
    /// Session mutation response.
    SessionMutateResponse = 203,
    /// Session rename request.
    SessionRenameRequest = 204,
    /// Session close request.
    SessionCloseRequest = 205,
    /// Controller takeover request.
    SessionTakeoverRequest = 206,
    /// Allocate a daemon-issued mutation replay lease.
    SessionOperationLeaseRequest = 207,
    /// Daemon-issued mutation replay lease.
    SessionOperationLeaseResponse = 208,
    /// Terminal attach request.
    TerminalAttachRequest = 300,
    /// Terminal full snapshot.
    TerminalSnapshot = 301,
    /// Terminal merged delta.
    TerminalDelta = 302,
    /// Terminal input.
    TerminalInput = 303,
    /// Terminal resize.
    TerminalResize = 304,
    /// Terminal detach.
    TerminalDetach = 305,
    /// Acknowledgement that a snapshot was applied atomically.
    TerminalSnapshotApplied = 306,
    /// Request for a snapshot from the client's known revision.
    TerminalSyncRequest = 307,
    /// Instruction to resynchronize from the latest revision.
    TerminalSyncRequired = 308,
    /// A controller attachment lost its lease to a takeover.
    TerminalLeaseLost = 309,
    /// The root shell and session have ended.
    TerminalSessionEnded = 310,
    /// Latest daemon-owned remote attachment transport state for one local view.
    TerminalTransportStateEvent = 311,
}

impl WireKind {
    /// Returns whether the kind uses the stricter control-payload limit.
    #[must_use]
    pub const fn is_control(self) -> bool {
        !matches!(self, Self::TerminalSnapshot | Self::TerminalDelta)
    }

    /// Returns whether the kind is a short pair/hello transport frame.
    #[must_use]
    pub const fn is_pair_hello(self) -> bool {
        matches!(
            self,
            Self::PairBegin
                | Self::PairChallenge
                | Self::PairProof
                | Self::PairAccepted
                | Self::ConnectionHello
                | Self::ConnectionWelcome
        )
    }

    /// Returns the maximum admitted control payload bytes for this kind.
    #[must_use]
    pub const fn max_control_payload_bytes(self) -> usize {
        if self.is_pair_hello() {
            MAX_PAIR_HELLO_FRAME_BYTES
        } else {
            MAX_CONTROL_PAYLOAD_BYTES
        }
    }
}

impl TryFrom<u32> for WireKind {
    type Error = ProtocolError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        let kind = match value {
            1 => Self::LocalReadinessRequest,
            2 => Self::LocalReadinessResponse,
            3 => Self::LocalStatusRequest,
            4 => Self::LocalStatusResponse,
            5 => Self::LocalValidateSetupRequest,
            6 => Self::LocalValidateSetupResponse,
            7 => Self::LocalStopRequest,
            8 => Self::LocalStopResponse,
            9 => Self::LocalUpdatePreflightRequest,
            10 => Self::LocalUpdatePreflightResponse,
            11 => Self::ServiceErrorResponse,
            12 => Self::LocalPairCreateRequest,
            13 => Self::LocalPairCreateResponse,
            14 => Self::LocalPairAcceptRequest,
            15 => Self::LocalPairAcceptResponse,
            16 => Self::LocalDeviceListRequest,
            17 => Self::LocalDeviceListResponse,
            18 => Self::LocalDeviceRenameRequest,
            19 => Self::LocalDeviceRenameResponse,
            20 => Self::LocalDeviceRevokeRequest,
            21 => Self::LocalDeviceRevokeResponse,
            22 => Self::LocalTargetResolveRequest,
            23 => Self::LocalTargetResolveResponse,
            24 => Self::LocalSessionUnaryRequest,
            100 => Self::PairBegin,
            101 => Self::PairChallenge,
            102 => Self::PairProof,
            103 => Self::PairAccepted,
            104 => Self::ConnectionHello,
            105 => Self::ConnectionWelcome,
            200 => Self::SessionListRequest,
            201 => Self::SessionListResponse,
            202 => Self::SessionCreateRequest,
            203 => Self::SessionMutateResponse,
            204 => Self::SessionRenameRequest,
            205 => Self::SessionCloseRequest,
            206 => Self::SessionTakeoverRequest,
            207 => Self::SessionOperationLeaseRequest,
            208 => Self::SessionOperationLeaseResponse,
            300 => Self::TerminalAttachRequest,
            301 => Self::TerminalSnapshot,
            302 => Self::TerminalDelta,
            303 => Self::TerminalInput,
            304 => Self::TerminalResize,
            305 => Self::TerminalDetach,
            306 => Self::TerminalSnapshotApplied,
            307 => Self::TerminalSyncRequest,
            308 => Self::TerminalSyncRequired,
            309 => Self::TerminalLeaseLost,
            310 => Self::TerminalSessionEnded,
            311 => Self::TerminalTransportStateEvent,
            unknown => return Err(ProtocolError::UnknownKind(unknown)),
        };
        Ok(kind)
    }
}

/// A decoded frame after major, kind, and size validation.
#[derive(Clone, Eq, PartialEq)]
pub struct DecodedFrame {
    /// Stable message kind.
    pub kind: WireKind,
    /// Unary request correlation ID.
    pub request_id: u64,
    /// Relative request deadline in milliseconds; zero selects service default.
    pub deadline_ms: u32,
    /// Still-encoded concrete protobuf message.
    pub payload: Vec<u8>,
}

impl fmt::Debug for DecodedFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecodedFrame")
            .field("kind", &self.kind)
            .field("request_id", &self.request_id)
            .field("deadline_ms", &self.deadline_ms)
            .field("payload", &"[REDACTED]")
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

impl DecodedFrame {
    /// Decodes the concrete message after checking its expected kind.
    pub fn decode_message<M: Message + Default>(
        &self,
        expected: WireKind,
    ) -> Result<M, ProtocolError> {
        if self.kind != expected {
            return Err(ProtocolError::UnexpectedKind {
                expected,
                actual: self.kind,
            });
        }
        M::decode(self.payload.as_slice()).map_err(ProtocolError::MalformedProtobuf)
    }
}

/// Errors returned by the zterm frame/protobuf validation boundary.
#[derive(Debug)]
pub enum ProtocolError {
    /// Length prefix exceeded unsigned 64-bit varint syntax.
    MalformedVarint,
    /// Stream ended in a prefix or frame body.
    TruncatedFrame,
    /// Frame length exceeded the 8 MiB bound before allocation.
    FrameTooLarge(usize),
    /// Control payload exceeded its per-kind bound before concrete decoding.
    ControlPayloadTooLarge(usize),
    /// `WireFrame` or concrete payload protobuf was malformed.
    MalformedProtobuf(prost::DecodeError),
    /// Peer uses an incompatible product wire major.
    WireMajorMismatch {
        /// Local supported major.
        expected: u32,
        /// Peer-provided major.
        actual: u32,
    },
    /// Numeric message kind is not registered.
    UnknownKind(u32),
    /// Caller attempted to decode a frame as another known kind.
    UnexpectedKind {
        /// Kind required by the typed decoder.
        expected: WireKind,
        /// Kind present in the frame.
        actual: WireKind,
    },
    /// Domain ID field had the wrong fixed width.
    InvalidIdentifier(IdLengthError),
    /// A wire viewport did not fit the zterm-owned non-zero `u16` boundary.
    InvalidTerminalSize {
        /// Wire row count.
        rows: u32,
        /// Wire column count.
        columns: u32,
    },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedVarint => write!(formatter, "malformed frame length varint"),
            Self::TruncatedFrame => write!(formatter, "stream ended inside a frame"),
            Self::FrameTooLarge(length) => {
                write!(
                    formatter,
                    "frame length {length} exceeds {MAX_FRAME_BYTES} bytes"
                )
            }
            Self::ControlPayloadTooLarge(length) => {
                write!(
                    formatter,
                    "control payload length {length} exceeds its per-kind bound"
                )
            }
            Self::MalformedProtobuf(error) => write!(formatter, "malformed protobuf: {error}"),
            Self::WireMajorMismatch { expected, actual } => write!(
                formatter,
                "wire major mismatch: local {expected}, peer {actual}"
            ),
            Self::UnknownKind(kind) => write!(formatter, "unknown wire message kind {kind}"),
            Self::UnexpectedKind { expected, actual } => {
                write!(formatter, "expected {expected:?}, got {actual:?}")
            }
            Self::InvalidIdentifier(error) => error.fmt(formatter),
            Self::InvalidTerminalSize { rows, columns } => {
                write!(formatter, "invalid terminal viewport {columns}x{rows}")
            }
        }
    }
}

impl std::error::Error for ProtocolError {}

/// Incremental decoder for `varint length + WireFrame` streams.
pub struct FrameDecoder {
    prefix: [u8; MAX_VARINT_BYTES],
    prefix_len: usize,
    body: Vec<u8>,
    expected_body: Option<usize>,
    maximum_body_bytes: usize,
}

impl fmt::Debug for FrameDecoder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrameDecoder")
            .field("prefix_bytes", &self.prefix_len)
            .field("buffered_body_bytes", &self.body.len())
            .field("expected_body_bytes", &self.expected_body)
            .field("maximum_body_bytes", &self.maximum_body_bytes)
            .finish()
    }
}

impl Drop for FrameDecoder {
    fn drop(&mut self) {
        self.prefix.zeroize();
        self.prefix_len = 0;
        self.body.zeroize();
        self.expected_body = None;
        self.maximum_body_bytes = 0;
    }
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameDecoder {
    /// Creates an empty incremental decoder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            prefix: [0; MAX_VARINT_BYTES],
            prefix_len: 0,
            body: Vec::new(),
            expected_body: None,
            maximum_body_bytes: MAX_FRAME_BYTES,
        }
    }

    /// Creates an incremental decoder with a stricter frame-body ceiling.
    ///
    /// Transport handshakes use this constructor before authentication so the
    /// length prefix cannot cause an allocation up to the broader 8 MiB service
    /// limit. The requested limit is always capped by [`MAX_FRAME_BYTES`].
    #[must_use]
    pub const fn with_maximum_body_bytes(maximum_body_bytes: usize) -> Self {
        Self {
            prefix: [0; MAX_VARINT_BYTES],
            prefix_len: 0,
            body: Vec::new(),
            expected_body: None,
            maximum_body_bytes: if maximum_body_bytes < MAX_FRAME_BYTES {
                maximum_body_bytes
            } else {
                MAX_FRAME_BYTES
            },
        }
    }

    /// Feeds arbitrary stream bytes and returns every complete validated frame.
    pub fn feed(&mut self, mut input: &[u8]) -> Result<Vec<DecodedFrame>, ProtocolError> {
        let mut frames = Vec::new();
        while !input.is_empty() {
            if self.expected_body.is_none() {
                let byte = input[0];
                input = &input[1..];
                if self.prefix_len == MAX_VARINT_BYTES {
                    return Err(ProtocolError::MalformedVarint);
                }
                self.prefix[self.prefix_len] = byte;
                self.prefix_len += 1;
                if byte & 0x80 == 0 {
                    let length = decode_varint(&self.prefix[..self.prefix_len])?;
                    let length = usize::try_from(length)
                        .map_err(|_| ProtocolError::FrameTooLarge(usize::MAX))?;
                    if length > self.maximum_body_bytes {
                        return Err(ProtocolError::FrameTooLarge(length));
                    }
                    self.body = Vec::with_capacity(length);
                    self.expected_body = Some(length);
                } else if self.prefix_len == MAX_VARINT_BYTES {
                    return Err(ProtocolError::MalformedVarint);
                }
            }

            if let Some(expected) = self.expected_body {
                let needed = expected.saturating_sub(self.body.len());
                let taken = needed.min(input.len());
                self.body.extend_from_slice(&input[..taken]);
                input = &input[taken..];
                if self.body.len() == expected {
                    let body = Zeroizing::new(std::mem::take(&mut self.body));
                    self.expected_body = None;
                    self.prefix_len = 0;
                    frames.push(decode_wire_frame(body.as_slice())?);
                }
            }
        }
        Ok(frames)
    }

    /// Verifies that a completed byte stream ended on a frame boundary.
    pub fn finish(self) -> Result<(), ProtocolError> {
        if self.prefix_len == 0 && self.expected_body.is_none() {
            Ok(())
        } else {
            Err(ProtocolError::TruncatedFrame)
        }
    }
}

/// Encodes one concrete protobuf message into the bounded stream format.
pub fn encode_message<M: Message>(
    kind: WireKind,
    request_id: u64,
    deadline_ms: u32,
    message: &M,
) -> Result<Vec<u8>, ProtocolError> {
    encode_payload(kind, request_id, deadline_ms, message.encode_to_vec())
}

/// Encodes an already serialized concrete payload into the bounded stream format.
pub fn encode_payload(
    kind: WireKind,
    request_id: u64,
    deadline_ms: u32,
    payload: Vec<u8>,
) -> Result<Vec<u8>, ProtocolError> {
    validate_payload_limit(kind, payload.len())?;
    let wire = v1::WireFrame {
        wire_major: WIRE_MAJOR,
        kind: kind as u32,
        payload,
        request_id,
        deadline_ms,
    };
    let body = wire.encode_to_vec();
    if body.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge(body.len()));
    }
    let mut output = Vec::with_capacity(MAX_VARINT_BYTES + body.len());
    encode_varint(body.len() as u64, &mut output);
    output.extend_from_slice(&body);
    Ok(output)
}

fn decode_wire_frame(body: &[u8]) -> Result<DecodedFrame, ProtocolError> {
    let wire = v1::WireFrame::decode(body).map_err(ProtocolError::MalformedProtobuf)?;
    if wire.wire_major != WIRE_MAJOR {
        return Err(ProtocolError::WireMajorMismatch {
            expected: WIRE_MAJOR,
            actual: wire.wire_major,
        });
    }
    let kind = WireKind::try_from(wire.kind)?;
    validate_payload_limit(kind, wire.payload.len())?;
    Ok(DecodedFrame {
        kind,
        request_id: wire.request_id,
        deadline_ms: wire.deadline_ms,
        payload: wire.payload,
    })
}

fn validate_payload_limit(kind: WireKind, length: usize) -> Result<(), ProtocolError> {
    let maximum = kind.max_control_payload_bytes();
    if kind.is_control() && length > maximum {
        Err(ProtocolError::ControlPayloadTooLarge(length))
    } else {
        Ok(())
    }
}

fn encode_varint(mut value: u64, output: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            output.push(byte);
            return;
        }
        output.push(byte | 0x80);
    }
}

fn decode_varint(bytes: &[u8]) -> Result<u64, ProtocolError> {
    let mut value = 0_u64;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if index == 9 && byte > 1 {
            return Err(ProtocolError::MalformedVarint);
        }
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            if index > 0 && byte == 0 {
                return Err(ProtocolError::MalformedVarint);
            }
            return Ok(value);
        }
    }
    Err(ProtocolError::MalformedVarint)
}

impl From<DeviceId> for v1::DeviceId {
    fn from(value: DeviceId) -> Self {
        Self {
            value: value.to_bytes().to_vec(),
        }
    }
}

impl TryFrom<v1::DeviceId> for DeviceId {
    type Error = ProtocolError;

    fn try_from(value: v1::DeviceId) -> Result<Self, Self::Error> {
        Self::from_bytes(&value.value).map_err(ProtocolError::InvalidIdentifier)
    }
}

impl From<SessionId> for v1::SessionId {
    fn from(value: SessionId) -> Self {
        Self {
            value: value.to_bytes().to_vec(),
        }
    }
}

impl TryFrom<v1::SessionId> for SessionId {
    type Error = ProtocolError;

    fn try_from(value: v1::SessionId) -> Result<Self, Self::Error> {
        Self::from_bytes(&value.value).map_err(ProtocolError::InvalidIdentifier)
    }
}

impl From<AttachmentId> for v1::AttachmentId {
    fn from(value: AttachmentId) -> Self {
        Self {
            value: value.to_bytes().to_vec(),
        }
    }
}

impl TryFrom<v1::AttachmentId> for AttachmentId {
    type Error = ProtocolError;

    fn try_from(value: v1::AttachmentId) -> Result<Self, Self::Error> {
        Self::from_bytes(&value.value).map_err(ProtocolError::InvalidIdentifier)
    }
}

impl From<ResumeViewId> for v1::ResumeViewId {
    fn from(value: ResumeViewId) -> Self {
        Self {
            value: value.to_bytes().to_vec(),
        }
    }
}

impl TryFrom<v1::ResumeViewId> for ResumeViewId {
    type Error = ProtocolError;

    fn try_from(value: v1::ResumeViewId) -> Result<Self, Self::Error> {
        Self::from_bytes(&value.value).map_err(ProtocolError::InvalidIdentifier)
    }
}

impl From<OperationId> for v1::OperationId {
    fn from(value: OperationId) -> Self {
        Self {
            lease_ordinal: value.lease.ordinal,
            sequence: value.sequence,
            daemon_incarnation: value.lease.daemon_incarnation.to_bytes().to_vec(),
        }
    }
}

impl TryFrom<v1::OperationId> for OperationId {
    type Error = ProtocolError;

    fn try_from(value: v1::OperationId) -> Result<Self, Self::Error> {
        Ok(Self {
            lease: OperationLease {
                daemon_incarnation: DaemonIncarnation::from_bytes(&value.daemon_incarnation)
                    .map_err(ProtocolError::InvalidIdentifier)?,
                ordinal: value.lease_ordinal,
            },
            sequence: value.sequence,
        })
    }
}

impl From<OperationLease> for v1::OperationLease {
    fn from(value: OperationLease) -> Self {
        Self {
            daemon_incarnation: value.daemon_incarnation.to_bytes().to_vec(),
            ordinal: value.ordinal,
        }
    }
}

impl TryFrom<v1::OperationLease> for OperationLease {
    type Error = ProtocolError;

    fn try_from(value: v1::OperationLease) -> Result<Self, Self::Error> {
        Ok(Self {
            daemon_incarnation: DaemonIncarnation::from_bytes(&value.daemon_incarnation)
                .map_err(ProtocolError::InvalidIdentifier)?,
            ordinal: value.ordinal,
        })
    }
}

impl From<TerminalSize> for v1::TerminalViewport {
    fn from(value: TerminalSize) -> Self {
        Self {
            rows: u32::from(value.rows),
            columns: u32::from(value.columns),
        }
    }
}

impl TryFrom<v1::TerminalViewport> for TerminalSize {
    type Error = ProtocolError;

    fn try_from(value: v1::TerminalViewport) -> Result<Self, Self::Error> {
        let rows = u16::try_from(value.rows).map_err(|_| ProtocolError::InvalidTerminalSize {
            rows: value.rows,
            columns: value.columns,
        })?;
        let columns =
            u16::try_from(value.columns).map_err(|_| ProtocolError::InvalidTerminalSize {
                rows: value.rows,
                columns: value.columns,
            })?;
        if rows == 0 || columns == 0 {
            return Err(ProtocolError::InvalidTerminalSize {
                rows: value.rows,
                columns: value.columns,
            });
        }
        Ok(TerminalSize::new(rows, columns))
    }
}

impl From<ActiveScreen> for v1::TerminalActiveScreen {
    fn from(value: ActiveScreen) -> Self {
        match value {
            ActiveScreen::Main => Self::Main,
            ActiveScreen::Alternate => Self::Alternate,
        }
    }
}

impl From<TerminalModes> for v1::TerminalModes {
    fn from(value: TerminalModes) -> Self {
        Self {
            application_keypad: value.application_keypad,
            application_cursor: value.application_cursor,
            bracketed_paste: value.bracketed_paste,
            focus_reporting: value.focus_reporting,
            mouse_mode: match value.mouse_mode {
                TerminalMouseMode::None => 0,
                TerminalMouseMode::Press => 1,
                TerminalMouseMode::PressRelease => 2,
                TerminalMouseMode::ButtonMotion => 3,
                TerminalMouseMode::AnyMotion => 4,
            },
            mouse_encoding: match value.mouse_encoding {
                TerminalMouseEncoding::Default => 0,
                TerminalMouseEncoding::Utf8 => 1,
                TerminalMouseEncoding::Sgr => 2,
            },
        }
    }
}

/// Projects one host snapshot into its wire message without reparsing ANSI.
#[must_use]
pub fn terminal_snapshot_message(
    session_id: SessionId,
    attachment_id: AttachmentId,
    mut snapshot: TerminalSnapshot,
) -> v1::TerminalSnapshot {
    let _ = snapshot.limit_ansi_payload(MAX_FRAME_BYTES - TERMINAL_SNAPSHOT_FRAME_HEADROOM);
    v1::TerminalSnapshot {
        session_id: Some(session_id.into()),
        attachment_id: Some(attachment_id.into()),
        revision: snapshot.revision.get(),
        rows: u32::from(snapshot.size.rows),
        columns: u32::from(snapshot.size.columns),
        screen_ansi: snapshot.screen_ansi,
        recent_history_ansi: snapshot.recent_history_ansi,
        active_screen: v1::TerminalActiveScreen::from(snapshot.active_screen) as i32,
        modes: Some(snapshot.modes.into()),
    }
}

/// Projects one merged host delta into its wire message.
#[must_use]
pub fn terminal_delta_message(
    attachment_id: AttachmentId,
    delta: TerminalDelta,
) -> v1::TerminalDelta {
    v1::TerminalDelta {
        from_revision: delta.from_revision.get(),
        to_revision: delta.to_revision.get(),
        ansi: delta.ansi,
        rows: u32::from(delta.size.rows),
        columns: u32::from(delta.size.columns),
        active_screen: v1::TerminalActiveScreen::from(delta.active_screen) as i32,
        modes: Some(delta.modes.into()),
        attachment_id: Some(attachment_id.into()),
    }
}

/// Fixed text prefix before the base64url-no-pad ticket payload.
pub const PAIR_TICKET_PREFIX: &str = "zterm-pair-v1:";
/// Version of the persisted relay route cache.
pub const RELAY_ROUTE_CACHE_VERSION: u32 = 1;
/// Maximum bytes in a persisted relay route cache blob before decoding.
///
/// The product ceiling is four 2048-byte relay URLs plus framing, so 16 KiB is
/// a generous bound that still rejects a malformed or hostile oversized blob.
pub const MAX_RELAY_ROUTE_CACHE_BYTES: usize = 16 * 1024;
/// Maximum hello or pair frame body bytes for kinds 100-105.
pub const MAX_PAIR_HELLO_FRAME_BYTES: usize = zterm_core::MAX_PAIR_HELLO_FRAME_BYTES;
/// Maximum total bytes exchanged in one pairing handshake.
pub const MAX_PAIR_HANDSHAKE_BYTES: usize = zterm_core::MAX_PAIR_HANDSHAKE_BYTES;

/// Failure while encoding or decoding a pairing ticket text.
#[derive(Debug)]
pub enum TicketTextError {
    /// Text did not begin with the fixed pairing prefix.
    MissingPrefix,
    /// Ticket text exceeded the 16 KiB bound before any allocation.
    TooLong {
        /// Observed byte count.
        actual: usize,
    },
    /// Base64url payload was malformed.
    InvalidBase64(base64::DecodeError),
    /// Decoded protobuf was malformed.
    InvalidProtobuf(prost::DecodeError),
    /// A fixed-width domain field had the wrong byte length.
    InvalidIdentifier(IdLengthError),
    /// The pairing secret had the wrong byte length.
    InvalidSecret(PairSecretError),
    /// A relay hint URL was invalid.
    InvalidRelayHint(RelayHintError),
    /// Ticket fields failed their product contract.
    InvalidTicket(PairTicketError),
}

impl fmt::Display for TicketTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPrefix => {
                write!(
                    formatter,
                    "ticket text must start with {PAIR_TICKET_PREFIX:?}"
                )
            }
            Self::TooLong { actual } => write!(
                formatter,
                "ticket text length {actual} exceeds {MAX_TICKET_TEXT_BYTES} bytes"
            ),
            Self::InvalidBase64(error) => write!(formatter, "invalid ticket base64url: {error}"),
            Self::InvalidProtobuf(error) => write!(formatter, "invalid ticket protobuf: {error}"),
            Self::InvalidIdentifier(error) => error.fmt(formatter),
            Self::InvalidSecret(error) => error.fmt(formatter),
            Self::InvalidRelayHint(error) => error.fmt(formatter),
            Self::InvalidTicket(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TicketTextError {}

/// Failure while encoding or decoding a persisted relay route cache.
#[derive(Debug)]
pub enum RouteCacheError {
    /// Cache protobuf bytes were malformed.
    Malformed(prost::DecodeError),
    /// Cache version is not recognized; the caller ignores it with a diagnostic.
    UnsupportedVersion {
        /// Observed cache version.
        actual: u32,
    },
    /// A cached relay URL was invalid.
    InvalidRelayHint(RelayHintError),
    /// A persisted verified route cache contained no relay URL.
    MissingUrl,
    /// The cache advertised more relay URLs than the product bound.
    TooManyUrls {
        /// Observed count.
        actual: usize,
    },
    /// The cache byte blob exceeded its pre-decode ceiling.
    TooLarge {
        /// Observed byte count.
        actual: usize,
    },
}

impl fmt::Display for RouteCacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(error) => write!(formatter, "malformed relay route cache: {error}"),
            Self::UnsupportedVersion { actual } => {
                write!(formatter, "unsupported relay route cache version {actual}")
            }
            Self::InvalidRelayHint(error) => error.fmt(formatter),
            Self::MissingUrl => write!(formatter, "relay route cache must contain a relay URL"),
            Self::TooManyUrls { actual } => {
                write!(
                    formatter,
                    "relay route cache advertised too many URLs ({actual})"
                )
            }
            Self::TooLarge { actual } => write!(
                formatter,
                "relay route cache length {actual} exceeds {MAX_RELAY_ROUTE_CACHE_BYTES} bytes"
            ),
        }
    }
}

impl std::error::Error for RouteCacheError {}

impl From<(&PairTicketFields, &PairSecret)> for v1::PairTicketV1 {
    fn from((fields, secret): (&PairTicketFields, &PairSecret)) -> Self {
        Self {
            format_version: fields.format_version(),
            host_device_id: Some(fields.host_device_id().into()),
            host_name: fields.host_name().to_owned(),
            relay_urls: fields
                .relay_hints()
                .iter()
                .map(|hint| hint.as_str().to_owned())
                .collect(),
            offer_id: fields.offer_id().to_bytes().to_vec(),
            secret: secret.as_bytes().to_vec(),
            expires_at_unix: fields.expires_at_unix(),
        }
    }
}

impl TryFrom<v1::PairTicketV1> for (PairTicketFields, PairSecret) {
    type Error = TicketTextError;

    fn try_from(value: v1::PairTicketV1) -> Result<Self, Self::Error> {
        let v1::PairTicketV1 {
            format_version,
            host_device_id,
            host_name,
            relay_urls,
            offer_id,
            secret,
            expires_at_unix,
        } = value;
        // Move the raw secret into zeroizing ownership before any fallible
        // validation, so every early return scrubs the bytes.
        let secret: Zeroizing<Vec<u8>> = Zeroizing::new(secret);
        let host_device_bytes = host_device_id.unwrap_or_default().value;
        let host_device_id =
            DeviceId::from_bytes(&host_device_bytes).map_err(TicketTextError::InvalidIdentifier)?;
        let offer_id = zterm_core::PairOfferId::from_bytes(&offer_id)
            .map_err(TicketTextError::InvalidIdentifier)?;
        let secret =
            PairSecret::from_slice(secret.as_slice()).map_err(TicketTextError::InvalidSecret)?;
        let relay_hints = relay_urls
            .iter()
            .map(|url| RelayHint::new(url.clone()).map_err(TicketTextError::InvalidRelayHint))
            .collect::<Result<Vec<_>, _>>()?;
        let fields = PairTicketFields::new(
            format_version,
            host_device_id,
            host_name,
            relay_hints,
            offer_id,
            expires_at_unix,
        )
        .map_err(TicketTextError::InvalidTicket)?;
        Ok((fields, secret))
    }
}

/// Encodes a pairing ticket as `zterm-pair-v1:` + base64url-no-pad text.
///
/// The secret-bearing fields are already validated, so a well-formed ticket
/// always fits the 16 KiB text bound. Every temporary prost/encoded secret
/// buffer is zeroized before the function returns; only the intended ticket
/// text (the caller's bearer value) is produced.
#[must_use]
pub fn encode_pair_ticket(fields: &PairTicketFields, secret: &PairSecret) -> String {
    let mut message = v1::PairTicketV1::from((fields, secret));
    let encoded: Zeroizing<Vec<u8>> = Zeroizing::new(message.encode_to_vec());
    message.secret.zeroize();
    // The base64 string is a second bearer copy; zeroize it once copied out.
    let encoded_text: Zeroizing<String> = Zeroizing::new(URL_SAFE_NO_PAD.encode(&*encoded));
    let mut text = String::with_capacity(PAIR_TICKET_PREFIX.len() + encoded_text.len());
    text.push_str(PAIR_TICKET_PREFIX);
    text.push_str(encoded_text.as_str());
    debug_assert!(text.len() <= MAX_TICKET_TEXT_BYTES);
    text
}

/// Decodes and validates a `zterm-pair-v1:` ticket text.
///
/// The 16 KiB bound is enforced before base64 decoding, and every secret or
/// decoded buffer is zeroized on drop or immediately after extraction.
pub fn decode_pair_ticket(text: &str) -> Result<(PairTicketFields, PairSecret), TicketTextError> {
    if text.len() > MAX_TICKET_TEXT_BYTES {
        return Err(TicketTextError::TooLong { actual: text.len() });
    }
    let encoded = text
        .strip_prefix(PAIR_TICKET_PREFIX)
        .ok_or(TicketTextError::MissingPrefix)?;
    let decoded: Zeroizing<Vec<u8>> = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(TicketTextError::InvalidBase64)?,
    );
    let message =
        v1::PairTicketV1::decode(decoded.as_slice()).map_err(TicketTextError::InvalidProtobuf)?;
    (message).try_into()
}

/// Encodes a versioned relay-only route cache, rejecting an oversized slice.
pub fn encode_relay_route_cache(urls: &[RelayHint]) -> Result<Vec<u8>, RouteCacheError> {
    if urls.is_empty() {
        return Err(RouteCacheError::MissingUrl);
    }
    if urls.len() > zterm_core::MAX_RELAY_HINTS {
        return Err(RouteCacheError::TooManyUrls { actual: urls.len() });
    }
    Ok(v1::RelayRouteCacheV1 {
        format_version: RELAY_ROUTE_CACHE_VERSION,
        relay_urls: urls.iter().map(|hint| hint.as_str().to_owned()).collect(),
    }
    .encode_to_vec())
}

/// Decodes a versioned relay-only route cache; unknown versions, oversized
/// blobs, and excess URLs fail with a typed diagnostic and are never migrated
/// or used.
pub fn decode_relay_route_cache(bytes: &[u8]) -> Result<Vec<RelayHint>, RouteCacheError> {
    if bytes.len() > MAX_RELAY_ROUTE_CACHE_BYTES {
        return Err(RouteCacheError::TooLarge {
            actual: bytes.len(),
        });
    }
    let cache = v1::RelayRouteCacheV1::decode(bytes).map_err(RouteCacheError::Malformed)?;
    if cache.format_version != RELAY_ROUTE_CACHE_VERSION {
        return Err(RouteCacheError::UnsupportedVersion {
            actual: cache.format_version,
        });
    }
    if cache.relay_urls.is_empty() {
        return Err(RouteCacheError::MissingUrl);
    }
    if cache.relay_urls.len() > zterm_core::MAX_RELAY_HINTS {
        return Err(RouteCacheError::TooManyUrls {
            actual: cache.relay_urls.len(),
        });
    }
    cache
        .relay_urls
        .iter()
        .map(|url| RelayHint::new(url.clone()).map_err(RouteCacheError::InvalidRelayHint))
        .collect()
}

/// Failure while converting a wire message into a validated domain value.
#[derive(Debug)]
pub enum WireFieldError {
    /// A fixed-width field had the wrong byte length.
    InvalidIdentifier(IdLengthError),
    /// A pair handshake field failed its product contract.
    InvalidPair(PairTicketError),
    /// A connection handshake field failed its product contract.
    InvalidConnection(ConnectionError),
    /// A device alias failed its product contract.
    InvalidAlias(DeviceAliasError),
    /// A device summary projection was internally inconsistent.
    InvalidSummary(DeviceSummaryError),
    /// A device projection used an unspecified or unknown authorization status.
    InvalidAuthStatus {
        /// Observed protobuf enum value.
        actual: i32,
    },
    /// An authorization generation exceeded the SQLite signed 64-bit ceiling.
    InvalidGeneration {
        /// Observed generation value.
        actual: u64,
    },
}

impl fmt::Display for WireFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier(error) => error.fmt(formatter),
            Self::InvalidPair(error) => error.fmt(formatter),
            Self::InvalidConnection(error) => error.fmt(formatter),
            Self::InvalidAlias(error) => error.fmt(formatter),
            Self::InvalidSummary(error) => error.fmt(formatter),
            Self::InvalidAuthStatus { actual } => {
                write!(formatter, "invalid device authorization status {actual}")
            }
            Self::InvalidGeneration { actual } => write!(
                formatter,
                "authorization generation {actual} exceeds the signed 64-bit ceiling"
            ),
        }
    }
}

impl std::error::Error for WireFieldError {}

impl TryFrom<v1::PairBegin> for PairBegin {
    type Error = WireFieldError;

    fn try_from(value: v1::PairBegin) -> Result<Self, Self::Error> {
        let offer_id =
            PairOfferId::from_bytes(&value.offer_id).map_err(WireFieldError::InvalidIdentifier)?;
        let controller_nonce = PairNonce::from_bytes(&value.controller_nonce)
            .map_err(WireFieldError::InvalidIdentifier)?;
        PairBegin::new(
            offer_id,
            value.controller_name,
            controller_nonce,
            value.pair_protocol_version,
        )
        .map_err(WireFieldError::InvalidPair)
    }
}

impl From<&PairBegin> for v1::PairBegin {
    fn from(value: &PairBegin) -> Self {
        Self {
            offer_id: value.offer_id().to_bytes().to_vec(),
            controller_name: value.controller_name().to_owned(),
            controller_nonce: value.controller_nonce().to_bytes().to_vec(),
            pair_protocol_version: value.pair_protocol_version(),
        }
    }
}

impl TryFrom<v1::PairChallenge> for PairChallenge {
    type Error = WireFieldError;

    fn try_from(value: v1::PairChallenge) -> Result<Self, Self::Error> {
        let host_nonce =
            PairNonce::from_bytes(&value.host_nonce).map_err(WireFieldError::InvalidIdentifier)?;
        PairChallenge::new(host_nonce, value.selected_version, value.ticket_expiry_unix)
            .map_err(WireFieldError::InvalidPair)
    }
}

impl From<&PairChallenge> for v1::PairChallenge {
    fn from(value: &PairChallenge) -> Self {
        Self {
            host_nonce: value.host_nonce().to_bytes().to_vec(),
            selected_version: value.selected_version(),
            ticket_expiry_unix: value.ticket_expiry_unix(),
        }
    }
}

impl TryFrom<v1::PairProof> for PairProof {
    type Error = WireFieldError;

    fn try_from(value: v1::PairProof) -> Result<Self, Self::Error> {
        let proof = Zeroizing::new(value.controller_proof);
        PairProof::from_slice(&proof).map_err(WireFieldError::InvalidIdentifier)
    }
}

impl From<&PairProof> for v1::PairProof {
    fn from(value: &PairProof) -> Self {
        Self {
            controller_proof: value.as_bytes().to_vec(),
        }
    }
}

impl TryFrom<v1::PairAccepted> for PairAccepted {
    type Error = WireFieldError;

    fn try_from(value: v1::PairAccepted) -> Result<Self, Self::Error> {
        let v1::PairAccepted {
            authorization_generation,
            host_confirmation_proof,
            host_diagnostic_version,
        } = value;
        let proof = Zeroizing::new(host_confirmation_proof);
        let generation = AuthGeneration::new(authorization_generation).ok_or(
            WireFieldError::InvalidGeneration {
                actual: authorization_generation,
            },
        )?;
        let proof = PairProof::from_slice(&proof).map_err(WireFieldError::InvalidIdentifier)?;
        PairAccepted::from_proof(generation, proof, host_diagnostic_version)
            .map_err(WireFieldError::InvalidPair)
    }
}

impl From<&PairAccepted> for v1::PairAccepted {
    fn from(value: &PairAccepted) -> Self {
        Self {
            authorization_generation: value.authorization_generation().get(),
            host_confirmation_proof: value.host_confirmation_proof().to_vec(),
            host_diagnostic_version: value.host_diagnostic_version().to_owned(),
        }
    }
}

impl TryFrom<v1::ConnectionHello> for ConnectionHello {
    type Error = WireFieldError;

    fn try_from(value: v1::ConnectionHello) -> Result<Self, Self::Error> {
        let attempt_id = ConnectionAttemptId::from_bytes(&value.attempt_id)
            .map_err(WireFieldError::InvalidIdentifier)?;
        ConnectionHello::new(
            value.min_wire_major,
            value.max_wire_major,
            Capabilities::from_bits_retain(value.capabilities),
            attempt_id,
            value.initiator_display,
            value.initiator_build,
            value.initiator_platform,
        )
        .map_err(WireFieldError::InvalidConnection)
    }
}

impl From<&ConnectionHello> for v1::ConnectionHello {
    fn from(value: &ConnectionHello) -> Self {
        Self {
            min_wire_major: value.min_wire_major(),
            max_wire_major: value.max_wire_major(),
            capabilities: value.capabilities().bits(),
            attempt_id: value.attempt_id().to_bytes().to_vec(),
            initiator_display: value.initiator_display().to_owned(),
            initiator_build: value.initiator_build().to_owned(),
            initiator_platform: value.initiator_platform().to_owned(),
        }
    }
}

impl TryFrom<v1::ConnectionWelcome> for ConnectionWelcome {
    type Error = WireFieldError;

    fn try_from(value: v1::ConnectionWelcome) -> Result<Self, Self::Error> {
        let generation = AuthGeneration::new(value.accepted_authorization_generation).ok_or(
            WireFieldError::InvalidGeneration {
                actual: value.accepted_authorization_generation,
            },
        )?;
        ConnectionWelcome::new(
            value.wire_major,
            Capabilities::from_bits_retain(value.capabilities),
            value.responder_display,
            value.responder_build,
            value.responder_platform,
            generation,
        )
        .map_err(WireFieldError::InvalidConnection)
    }
}

impl From<&ConnectionWelcome> for v1::ConnectionWelcome {
    fn from(value: &ConnectionWelcome) -> Self {
        Self {
            wire_major: value.wire_major(),
            capabilities: value.capabilities().bits(),
            responder_display: value.responder_display().to_owned(),
            responder_build: value.responder_build().to_owned(),
            responder_platform: value.responder_platform().to_owned(),
            accepted_authorization_generation: value.accepted_authorization_generation().get(),
        }
    }
}

impl TryFrom<v1::LocalDeviceRenameRequest> for (DeviceId, DeviceAlias) {
    type Error = WireFieldError;

    fn try_from(value: v1::LocalDeviceRenameRequest) -> Result<Self, Self::Error> {
        let device_bytes = value.device_id.unwrap_or_default().value;
        let device_id =
            DeviceId::from_bytes(&device_bytes).map_err(WireFieldError::InvalidIdentifier)?;
        let alias = DeviceAlias::new(value.alias).map_err(WireFieldError::InvalidAlias)?;
        Ok((device_id, alias))
    }
}

impl TryFrom<v1::LocalDeviceRevokeRequest> for DeviceId {
    type Error = WireFieldError;

    fn try_from(value: v1::LocalDeviceRevokeRequest) -> Result<Self, Self::Error> {
        let device_bytes = value.device_id.unwrap_or_default().value;
        DeviceId::from_bytes(&device_bytes).map_err(WireFieldError::InvalidIdentifier)
    }
}

impl TryFrom<v1::DeviceSummary> for DeviceSummary {
    type Error = WireFieldError;

    fn try_from(value: v1::DeviceSummary) -> Result<Self, Self::Error> {
        let device_bytes = value.device_id.unwrap_or_default().value;
        let device_id =
            DeviceId::from_bytes(&device_bytes).map_err(WireFieldError::InvalidIdentifier)?;
        let generation =
            AuthGeneration::new(value.generation).ok_or(WireFieldError::InvalidGeneration {
                actual: value.generation,
            })?;
        let alias = if value.alias.is_empty() {
            None
        } else {
            Some(DeviceAlias::new(value.alias).map_err(WireFieldError::InvalidAlias)?)
        };
        let auth_status = match v1::DeviceAuthStatus::try_from(value.auth_status) {
            Ok(v1::DeviceAuthStatus::None) => AuthorizationStatus::None,
            Ok(v1::DeviceAuthStatus::Authorized) => AuthorizationStatus::Authorized,
            Ok(v1::DeviceAuthStatus::Revoked) => AuthorizationStatus::Revoked,
            Ok(v1::DeviceAuthStatus::Unspecified) | Err(_) => {
                return Err(WireFieldError::InvalidAuthStatus {
                    actual: value.auth_status,
                });
            }
        };
        DeviceSummary::new(
            device_id,
            value.outbound_known,
            alias,
            value.remote_name,
            value.route_verified,
            auth_status,
            generation,
            value.paired_at_unix,
            value.last_seen_at_unix,
            value.online,
            value.active_stream_count,
            value.remote_attachment_count,
        )
        .map_err(WireFieldError::InvalidSummary)
    }
}

impl From<&DeviceSummary> for v1::DeviceSummary {
    fn from(value: &DeviceSummary) -> Self {
        Self {
            device_id: Some(value.device_id().into()),
            outbound_known: value.outbound_known(),
            alias: value
                .alias()
                .map_or_else(String::new, |alias| alias.as_str().to_owned()),
            remote_name: value.remote_name().to_owned(),
            route_verified: value.route_verified(),
            auth_status: match value.auth_status() {
                AuthorizationStatus::None => v1::DeviceAuthStatus::None as i32,
                AuthorizationStatus::Authorized => v1::DeviceAuthStatus::Authorized as i32,
                AuthorizationStatus::Revoked => v1::DeviceAuthStatus::Revoked as i32,
            },
            generation: value.generation().get(),
            paired_at_unix: value.paired_at_unix(),
            last_seen_at_unix: value.last_seen_at_unix(),
            online: value.online(),
            active_stream_count: value.active_stream_count(),
            remote_attachment_count: value.remote_attachment_count(),
        }
    }
}

/// Failure while validating the shared local pairing operation identity.
#[derive(Debug)]
pub enum PairOperationError {
    /// The ephemeral operation ID had the wrong fixed byte width.
    InvalidOperationId(IdLengthError),
    /// The semantic fingerprint had the wrong fixed byte width.
    InvalidFingerprint(PairFingerprintError),
}

impl fmt::Display for PairOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOperationId(error) => error.fmt(formatter),
            Self::InvalidFingerprint(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PairOperationError {}

/// Validates the shared local pairing operation identity before allocation.
///
/// Both local pair create and accept carry a client-generated 16-byte
/// ephemeral operation ID and a bounded semantic fingerprint. This is the
/// single boundary that converts the raw wire bytes into validated domain
/// values, so the service never allocates replay state for a malformed request.
pub fn validate_pair_operation(
    ephemeral_operation_id: &[u8],
    fingerprint: &[u8],
) -> Result<(EphemeralOperationId, PairFingerprint), PairOperationError> {
    let operation_id = EphemeralOperationId::from_bytes(ephemeral_operation_id)
        .map_err(PairOperationError::InvalidOperationId)?;
    let fingerprint =
        PairFingerprint::from_slice(fingerprint).map_err(PairOperationError::InvalidFingerprint)?;
    Ok((operation_id, fingerprint))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_message_round_trip<M>(kind: WireKind, message: M)
    where
        M: Message + Default + PartialEq + fmt::Debug,
    {
        let bytes = encode_message(kind, 41, 9_000, &message).expect("bounded future frame");
        let mut decoder = FrameDecoder::new();
        let frames = decoder.feed(&bytes).expect("future frame decodes");
        decoder.finish().expect("future frame is complete");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].kind, kind);
        assert_eq!(frames[0].request_id, 41);
        assert_eq!(frames[0].deadline_ms, 9_000);
        let decoded: M = frames[0]
            .decode_message(kind)
            .expect("future message payload decodes");
        assert_eq!(decoded, message);
    }

    #[test]
    fn frame_round_trip_and_unknown_fields_are_compatible() {
        let message = v1::LocalStatusRequest {};
        let mut bytes = encode_message(WireKind::LocalStatusRequest, 17, 5_000, &message)
            .expect("bounded status frame");
        assert_eq!(
            bytes,
            [0x09, 0x08, 0x01, 0x10, 0x03, 0x20, 0x11, 0x28, 0x88, 0x27],
            "language-neutral v1 empty-status golden frame"
        );
        let body_length = bytes[0] as usize;
        assert!(body_length < 0x80, "fixture keeps one-byte prefix");
        bytes[0] = u8::try_from(body_length + 3).expect("small fixture body");
        bytes.extend_from_slice(&[0xf8, 0x07, 0x01]);

        let mut decoder = FrameDecoder::new();
        let frames = decoder
            .feed(&bytes)
            .expect("unknown protobuf field ignored");
        decoder.finish().expect("complete frame boundary");
        assert_eq!(frames.len(), 1);
        let decoded: v1::LocalStatusRequest = frames[0]
            .decode_message(WireKind::LocalStatusRequest)
            .expect("typed payload");
        assert_eq!(decoded, message);
        assert_eq!(frames[0].request_id, 17);
    }

    #[test]
    fn decoder_rejects_unknown_major_kind_and_incomplete_or_malformed_lengths() {
        let unknown_major = v1::WireFrame {
            wire_major: 2,
            kind: WireKind::LocalStatusRequest as u32,
            payload: Vec::new(),
            request_id: 1,
            deadline_ms: 0,
        };
        let mut bytes = Vec::new();
        let body = unknown_major.encode_to_vec();
        encode_varint(body.len() as u64, &mut bytes);
        bytes.extend_from_slice(&body);
        assert!(matches!(
            FrameDecoder::new().feed(&bytes),
            Err(ProtocolError::WireMajorMismatch { actual: 2, .. })
        ));

        let unknown_kind = v1::WireFrame {
            wire_major: WIRE_MAJOR,
            kind: 65_535,
            payload: Vec::new(),
            request_id: 1,
            deadline_ms: 0,
        };
        let body = unknown_kind.encode_to_vec();
        let mut bytes = Vec::new();
        encode_varint(body.len() as u64, &mut bytes);
        bytes.extend_from_slice(&body);
        assert!(matches!(
            FrameDecoder::new().feed(&bytes),
            Err(ProtocolError::UnknownKind(65_535))
        ));

        let mut truncated = FrameDecoder::new();
        assert!(truncated.feed(&[5, 1, 2]).is_ok());
        assert!(matches!(
            truncated.finish(),
            Err(ProtocolError::TruncatedFrame)
        ));
        assert!(matches!(
            FrameDecoder::new().feed(&[0x80; MAX_VARINT_BYTES]),
            Err(ProtocolError::MalformedVarint)
        ));
        assert!(matches!(
            FrameDecoder::new().feed(&[0x80, 0x00]),
            Err(ProtocolError::MalformedVarint)
        ));
        assert!(matches!(
            FrameDecoder::new().feed(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02]),
            Err(ProtocolError::MalformedVarint)
        ));
        assert!(matches!(
            FrameDecoder::new().feed(&[0xff, 0xff, 0xff, 0xff, 0x7f]),
            Err(ProtocolError::FrameTooLarge(_))
        ));
    }

    #[test]
    fn decoder_debug_redacts_an_incomplete_body() {
        const SENTINEL: &[u8] = b"PAIR-DECODER-SECRET-SENTINEL";
        let mut bytes = vec![64];
        bytes.extend_from_slice(SENTINEL);
        let mut decoder = FrameDecoder::new();
        decoder
            .feed(&bytes)
            .expect("incomplete bounded body remains buffered");

        let debug = format!("{decoder:?}");
        assert!(!debug.contains("PAIR-DECODER-SECRET-SENTINEL"));
        assert!(!debug.contains(&format!("{SENTINEL:?}")));
        assert!(debug.contains("buffered_body_bytes"));
    }

    #[test]
    fn local_session_forward_envelope_round_trips_and_redacts_inner_bytes() {
        const SENTINEL: &[u8] = b"REMOTE-SESSION-INNER-FRAME-SENTINEL";
        let message = v1::LocalSessionUnaryRequest {
            target_device_id: Some(DeviceId::from_array([7; 32]).into()),
            frame: SENTINEL.to_vec(),
        };
        let debug = format!("{message:?}");
        assert!(!debug.contains("REMOTE-SESSION-INNER-FRAME-SENTINEL"));
        assert!(debug.contains("frame_len"));
        assert_message_round_trip(WireKind::LocalSessionUnaryRequest, message);

        let bounded_tunnel = v1::LocalSessionUnaryRequest {
            target_device_id: Some(DeviceId::from_array([8; 32]).into()),
            frame: vec![0; MAX_CONTROL_PAYLOAD_BYTES],
        };
        assert!(matches!(
            encode_message(WireKind::LocalSessionUnaryRequest, 2, 0, &bounded_tunnel,),
            Err(ProtocolError::ControlPayloadTooLarge(_))
        ));
    }

    #[test]
    fn encoder_and_decoder_enforce_control_and_total_frame_limits() {
        let exact_control = encode_payload(
            WireKind::LocalStatusResponse,
            1,
            0,
            vec![0; MAX_CONTROL_PAYLOAD_BYTES],
        )
        .expect("exact control ceiling is accepted");
        assert_eq!(
            FrameDecoder::new()
                .feed(&exact_control)
                .expect("exact control frame decodes")
                .len(),
            1
        );
        assert!(matches!(
            encode_payload(
                WireKind::LocalStatusResponse,
                1,
                0,
                vec![0; MAX_CONTROL_PAYLOAD_BYTES + 1]
            ),
            Err(ProtocolError::ControlPayloadTooLarge(_))
        ));
        let oversized_control = v1::WireFrame {
            wire_major: WIRE_MAJOR,
            kind: WireKind::LocalStatusResponse as u32,
            payload: vec![0; MAX_CONTROL_PAYLOAD_BYTES + 1],
            request_id: 1,
            deadline_ms: 0,
        }
        .encode_to_vec();
        let mut oversized_control_frame = Vec::new();
        encode_varint(oversized_control.len() as u64, &mut oversized_control_frame);
        oversized_control_frame.extend_from_slice(&oversized_control);
        assert!(matches!(
            FrameDecoder::new().feed(&oversized_control_frame),
            Err(ProtocolError::ControlPayloadTooLarge(_))
        ));
        let terminal = encode_payload(
            WireKind::TerminalSnapshot,
            1,
            0,
            vec![0; MAX_CONTROL_PAYLOAD_BYTES + 1],
        )
        .expect("terminal data can use the frame ceiling");
        let mut decoder = FrameDecoder::new();
        assert_eq!(
            decoder
                .feed(&terminal)
                .expect("bounded terminal frame")
                .len(),
            1
        );

        let mut prefix = Vec::new();
        encode_varint((MAX_FRAME_BYTES + 1) as u64, &mut prefix);
        assert!(matches!(
            FrameDecoder::new().feed(&prefix),
            Err(ProtocolError::FrameTooLarge(_))
        ));
    }

    #[test]
    fn terminal_snapshot_conversion_crops_only_history_to_fit_one_frame() {
        let screen = b"host-screen".to_vec();
        let mut history = b"\x1b[m".to_vec();
        while history.len() <= MAX_FRAME_BYTES + 1024 {
            history.extend_from_slice(b"complete-history-line\r\n");
        }
        let message = terminal_snapshot_message(
            SessionId::from_array([1; 16]),
            AttachmentId::from_array([2; 16]),
            TerminalSnapshot {
                revision: zterm_core::Revision::new(7),
                size: TerminalSize::new(2, 20),
                active_screen: ActiveScreen::Main,
                screen_ansi: screen.clone(),
                recent_history_ansi: history,
                modes: TerminalModes::default(),
            },
        );
        assert_eq!(message.screen_ansi, screen);
        assert!(message.recent_history_ansi.starts_with(b"\x1b[m"));
        assert!(message.recent_history_ansi.ends_with(b"\r\n"));
        let frame = encode_message(WireKind::TerminalSnapshot, 1, 0, &message)
            .expect("bounded snapshot frame");
        assert!(frame.len() <= MAX_FRAME_BYTES + MAX_VARINT_BYTES);
    }

    #[test]
    fn fixed_width_domain_ids_validate_once_at_proto_boundary() {
        let device = DeviceId::from_array([42; 32]);
        let decoded = DeviceId::try_from(v1::DeviceId::from(device)).expect("valid device id");
        assert_eq!(decoded, device);
        assert!(matches!(
            DeviceId::try_from(v1::DeviceId { value: vec![0; 31] }),
            Err(ProtocolError::InvalidIdentifier(_))
        ));
    }

    #[test]
    fn wire_kind_registry_matches_the_proto_source_of_truth() {
        let kinds = [
            (
                WireKind::LocalReadinessRequest,
                v1::MessageKind::LocalReadinessRequest as u32,
            ),
            (
                WireKind::LocalReadinessResponse,
                v1::MessageKind::LocalReadinessResponse as u32,
            ),
            (
                WireKind::LocalStatusRequest,
                v1::MessageKind::LocalStatusRequest as u32,
            ),
            (
                WireKind::LocalStatusResponse,
                v1::MessageKind::LocalStatusResponse as u32,
            ),
            (
                WireKind::LocalValidateSetupRequest,
                v1::MessageKind::LocalValidateSetupRequest as u32,
            ),
            (
                WireKind::LocalValidateSetupResponse,
                v1::MessageKind::LocalValidateSetupResponse as u32,
            ),
            (
                WireKind::LocalStopRequest,
                v1::MessageKind::LocalStopRequest as u32,
            ),
            (
                WireKind::LocalStopResponse,
                v1::MessageKind::LocalStopResponse as u32,
            ),
            (
                WireKind::LocalUpdatePreflightRequest,
                v1::MessageKind::LocalUpdatePreflightRequest as u32,
            ),
            (
                WireKind::LocalUpdatePreflightResponse,
                v1::MessageKind::LocalUpdatePreflightResponse as u32,
            ),
            (
                WireKind::ServiceErrorResponse,
                v1::MessageKind::ServiceErrorResponse as u32,
            ),
            (
                WireKind::LocalPairCreateRequest,
                v1::MessageKind::LocalPairCreateRequest as u32,
            ),
            (
                WireKind::LocalPairCreateResponse,
                v1::MessageKind::LocalPairCreateResponse as u32,
            ),
            (
                WireKind::LocalPairAcceptRequest,
                v1::MessageKind::LocalPairAcceptRequest as u32,
            ),
            (
                WireKind::LocalPairAcceptResponse,
                v1::MessageKind::LocalPairAcceptResponse as u32,
            ),
            (
                WireKind::LocalDeviceListRequest,
                v1::MessageKind::LocalDeviceListRequest as u32,
            ),
            (
                WireKind::LocalDeviceListResponse,
                v1::MessageKind::LocalDeviceListResponse as u32,
            ),
            (
                WireKind::LocalDeviceRenameRequest,
                v1::MessageKind::LocalDeviceRenameRequest as u32,
            ),
            (
                WireKind::LocalDeviceRenameResponse,
                v1::MessageKind::LocalDeviceRenameResponse as u32,
            ),
            (
                WireKind::LocalDeviceRevokeRequest,
                v1::MessageKind::LocalDeviceRevokeRequest as u32,
            ),
            (
                WireKind::LocalDeviceRevokeResponse,
                v1::MessageKind::LocalDeviceRevokeResponse as u32,
            ),
            (
                WireKind::LocalTargetResolveRequest,
                v1::MessageKind::LocalTargetResolveRequest as u32,
            ),
            (
                WireKind::LocalTargetResolveResponse,
                v1::MessageKind::LocalTargetResolveResponse as u32,
            ),
            (
                WireKind::LocalSessionUnaryRequest,
                v1::MessageKind::LocalSessionUnaryRequest as u32,
            ),
            (WireKind::PairBegin, v1::MessageKind::PairBegin as u32),
            (
                WireKind::PairChallenge,
                v1::MessageKind::PairChallenge as u32,
            ),
            (WireKind::PairProof, v1::MessageKind::PairProof as u32),
            (WireKind::PairAccepted, v1::MessageKind::PairAccepted as u32),
            (
                WireKind::ConnectionHello,
                v1::MessageKind::ConnectionHello as u32,
            ),
            (
                WireKind::ConnectionWelcome,
                v1::MessageKind::ConnectionWelcome as u32,
            ),
            (
                WireKind::SessionListRequest,
                v1::MessageKind::SessionListRequest as u32,
            ),
            (
                WireKind::SessionListResponse,
                v1::MessageKind::SessionListResponse as u32,
            ),
            (
                WireKind::SessionCreateRequest,
                v1::MessageKind::SessionCreateRequest as u32,
            ),
            (
                WireKind::SessionMutateResponse,
                v1::MessageKind::SessionMutateResponse as u32,
            ),
            (
                WireKind::SessionRenameRequest,
                v1::MessageKind::SessionRenameRequest as u32,
            ),
            (
                WireKind::SessionCloseRequest,
                v1::MessageKind::SessionCloseRequest as u32,
            ),
            (
                WireKind::SessionTakeoverRequest,
                v1::MessageKind::SessionTakeoverRequest as u32,
            ),
            (
                WireKind::SessionOperationLeaseRequest,
                v1::MessageKind::SessionOperationLeaseRequest as u32,
            ),
            (
                WireKind::SessionOperationLeaseResponse,
                v1::MessageKind::SessionOperationLeaseResponse as u32,
            ),
            (
                WireKind::TerminalAttachRequest,
                v1::MessageKind::TerminalAttachRequest as u32,
            ),
            (
                WireKind::TerminalSnapshot,
                v1::MessageKind::TerminalSnapshot as u32,
            ),
            (
                WireKind::TerminalDelta,
                v1::MessageKind::TerminalDelta as u32,
            ),
            (
                WireKind::TerminalInput,
                v1::MessageKind::TerminalInput as u32,
            ),
            (
                WireKind::TerminalResize,
                v1::MessageKind::TerminalResize as u32,
            ),
            (
                WireKind::TerminalDetach,
                v1::MessageKind::TerminalDetach as u32,
            ),
            (
                WireKind::TerminalSnapshotApplied,
                v1::MessageKind::TerminalSnapshotApplied as u32,
            ),
            (
                WireKind::TerminalSyncRequest,
                v1::MessageKind::TerminalSyncRequest as u32,
            ),
            (
                WireKind::TerminalSyncRequired,
                v1::MessageKind::TerminalSyncRequired as u32,
            ),
            (
                WireKind::TerminalLeaseLost,
                v1::MessageKind::TerminalLeaseLost as u32,
            ),
            (
                WireKind::TerminalSessionEnded,
                v1::MessageKind::TerminalSessionEnded as u32,
            ),
            (
                WireKind::TerminalTransportStateEvent,
                v1::MessageKind::TerminalTransportStateEvent as u32,
            ),
        ];

        for (kind, proto_number) in kinds {
            assert_eq!(kind as u32, proto_number);
            assert_eq!(
                WireKind::try_from(proto_number).expect("registered kind"),
                kind
            );
        }
    }

    #[test]
    fn session_and_terminal_contract_shapes_round_trip() {
        let operation_id = Some(v1::OperationId {
            lease_ordinal: 7,
            sequence: 11,
            daemon_incarnation: vec![9; 16],
        });
        let target = Some(v1::TargetSelector {
            target: Some(v1::target_selector::Target::Device(v1::DeviceId {
                value: vec![7; DeviceId::LENGTH],
            })),
        });
        let session_id = Some(v1::SessionId { value: vec![3; 16] });
        let attachment_id = Some(v1::AttachmentId { value: vec![5; 16] });

        assert_message_round_trip(
            WireKind::SessionRenameRequest,
            v1::SessionRenameRequest {
                operation_id: operation_id.clone(),
                target: target.clone(),
                session_id: session_id.clone(),
                name: "build".to_owned(),
            },
        );
        assert_message_round_trip(
            WireKind::SessionCloseRequest,
            v1::SessionCloseRequest {
                operation_id: operation_id.clone(),
                target: target.clone(),
                session_id: session_id.clone(),
            },
        );
        assert_message_round_trip(
            WireKind::TerminalAttachRequest,
            v1::TerminalAttachRequest {
                target: target.clone(),
                session_id: session_id.clone(),
                takeover: true,
                session_name: String::new(),
                create_main: false,
                viewport: Some(v1::TerminalViewport {
                    rows: 40,
                    columns: 120,
                }),
                resume_view_id: Some(v1::ResumeViewId { value: vec![6; 16] }),
                known_revision: Some(12),
            },
        );
        assert_message_round_trip(
            WireKind::SessionTakeoverRequest,
            v1::SessionTakeoverRequest {
                operation_id,
                target,
                session_id,
                attachment_id: attachment_id.clone(),
            },
        );
        assert_message_round_trip(
            WireKind::TerminalSnapshotApplied,
            v1::TerminalSnapshotApplied {
                attachment_id: attachment_id.clone(),
                revision: 13,
            },
        );
        assert_message_round_trip(
            WireKind::TerminalSyncRequest,
            v1::TerminalSyncRequest {
                attachment_id: attachment_id.clone(),
                known_revision: 13,
            },
        );
        assert_message_round_trip(
            WireKind::TerminalDelta,
            v1::TerminalDelta {
                from_revision: 12,
                to_revision: 13,
                ansi: b"safe fixture".to_vec(),
                rows: 40,
                columns: 120,
                active_screen: v1::TerminalActiveScreen::Main as i32,
                modes: Some(v1::TerminalModes::default()),
                attachment_id: attachment_id.clone(),
            },
        );
        assert_message_round_trip(
            WireKind::TerminalSyncRequired,
            v1::TerminalSyncRequired {
                attachment_id,
                latest_revision: 17,
            },
        );
        assert_message_round_trip(
            WireKind::TerminalLeaseLost,
            v1::TerminalLeaseLost {
                attachment_id: Some(v1::AttachmentId { value: vec![5; 16] }),
                generation: 3,
            },
        );
        assert_message_round_trip(
            WireKind::TerminalSessionEnded,
            v1::TerminalSessionEnded {
                session_id: Some(v1::SessionId { value: vec![3; 16] }),
                attachment_id: Some(v1::AttachmentId { value: vec![5; 16] }),
                reason: v1::TerminalSessionEndReason::NaturalExit as i32,
                exit_code: 0,
                signal: String::new(),
            },
        );
        assert_message_round_trip(
            WireKind::TerminalTransportStateEvent,
            v1::TerminalTransportStateEvent {
                attachment_id: Some(v1::AttachmentId { value: vec![5; 16] }),
                state: v1::TerminalTransportState::Reconnecting as i32,
            },
        );
        assert_eq!(
            ResumeViewId::try_from(v1::ResumeViewId { value: vec![6; 16] })
                .expect("fixed-width resume view ID"),
            ResumeViewId::from_array([6; 16])
        );
        assert!(matches!(
            ResumeViewId::try_from(v1::ResumeViewId { value: vec![6; 15] }),
            Err(ProtocolError::InvalidIdentifier(_))
        ));
        assert_eq!(
            TerminalSize::try_from(v1::TerminalViewport {
                rows: 40,
                columns: 120,
            })
            .expect("bounded viewport"),
            TerminalSize::new(40, 120)
        );
        assert!(matches!(
            TerminalSize::try_from(v1::TerminalViewport {
                rows: 0,
                columns: 120,
            }),
            Err(ProtocolError::InvalidTerminalSize { .. })
        ));
    }
}
