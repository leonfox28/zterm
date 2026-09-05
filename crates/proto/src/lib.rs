//! Versioned protobuf DTOs and the one bounded zterm frame codec.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use prost::Message;
use zeroize::{Zeroize, Zeroizing};
use zterm_core::terminal::{
    ActiveScreen, TerminalCell, TerminalClipboardWrite, TerminalColor, TerminalCursor,
    TerminalHistoryWindowAnchor, TerminalHistoryWindowQuery, TerminalKeyboardFlags, TerminalModes,
    TerminalMouseEncoding, TerminalMouseMode, TerminalScrollMetrics, TerminalSize, TerminalStyle,
    TerminalSurface, TerminalSurfaceDelta, TerminalSurfaceError, TerminalSurfaceHistoryWindowFrame,
    TerminalSurfaceHistoryWindowResult, TerminalSurfaceRow, TerminalSurfaceRowPatch,
    TerminalSurfaceSnapshot, TerminalViewportDisposition,
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

/// Generated version-two protocol DTOs.
pub mod v2 {
    #![allow(missing_docs)]
    include!(concat!(env!("OUT_DIR"), "/zterm.v2.rs"));
}

impl fmt::Debug for v2::WireFrame {
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

impl fmt::Debug for v2::TerminalCell {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalCell")
            .field("contents", &"[REDACTED]")
            .field("contents_len", &self.contents.len())
            .field("wide", &self.wide)
            .field("wide_continuation", &self.wide_continuation)
            .field("style", &self.style)
            .finish()
    }
}

impl fmt::Debug for v2::TerminalClipboardWrite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalClipboardWrite")
            .field("attachment_id_present", &self.attachment_id.is_some())
            .field("text", &"[REDACTED]")
            .field("text_len", &self.text.len())
            .finish()
    }
}

impl fmt::Debug for v2::TerminalSurfaceRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSurfaceRow")
            .field("cell_count", &self.cells.len())
            .field("wrapped", &self.wrapped)
            .finish()
    }
}

impl fmt::Debug for v2::TerminalSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSurface")
            .field("row_count", &self.row_count)
            .field("column_count", &self.column_count)
            .field("active_screen", &self.active_screen)
            .field("encoded_rows", &self.rows.len())
            .field("cursor", &self.cursor)
            .field("modes", &self.modes)
            .field("scroll_metrics", &self.scroll_metrics)
            .finish()
    }
}

impl fmt::Debug for v2::TerminalSemanticSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSemanticSnapshot")
            .field("session_id_present", &self.session_id.is_some())
            .field("attachment_id_present", &self.attachment_id.is_some())
            .field("revision", &self.revision)
            .field("surface", &self.surface)
            .finish()
    }
}

impl fmt::Debug for v2::TerminalSemanticRowPatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSemanticRowPatch")
            .field("row", &self.row)
            .field("replacement", &self.replacement)
            .finish()
    }
}

impl fmt::Debug for v2::TerminalSemanticDelta {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSemanticDelta")
            .field("attachment_id_present", &self.attachment_id.is_some())
            .field("from_revision", &self.from_revision)
            .field("to_revision", &self.to_revision)
            .field("row_count", &self.row_count)
            .field("column_count", &self.column_count)
            .field("active_screen", &self.active_screen)
            .field("row_patch_count", &self.row_patches.len())
            .field("cursor", &self.cursor)
            .field("modes", &self.modes)
            .field("scroll_metrics", &self.scroll_metrics)
            .finish()
    }
}

impl fmt::Debug for v2::TerminalSemanticHistoryWindowFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSemanticHistoryWindowFrame")
            .field("attachment_id_present", &self.attachment_id.is_some())
            .field("outcome", &self.outcome)
            .field("disposition", &self.disposition)
            .field("anchor", &self.anchor)
            .field("target_offset_from_bottom", &self.target_offset_from_bottom)
            .field("first_row_from_live_top", &self.first_row_from_live_top)
            .field("row_count", &self.rows.len())
            .field("current_epoch", &self.current_epoch)
            .field("current_revision", &self.current_revision)
            .finish()
    }
}

impl fmt::Debug for v2::PairTicketV1 {
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

impl fmt::Debug for v2::PairBegin {
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

impl fmt::Debug for v2::PairChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairChallenge")
            .field("host_nonce", &"[REDACTED]")
            .field("selected_version", &self.selected_version)
            .field("ticket_expiry_unix", &self.ticket_expiry_unix)
            .finish()
    }
}

impl fmt::Debug for v2::PairProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairProof")
            .field("controller_proof", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for v2::PairAccepted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairAccepted")
            .field("authorization_generation", &self.authorization_generation)
            .field("host_confirmation_proof", &"[REDACTED]")
            .field("host_diagnostic_version", &self.host_diagnostic_version)
            .finish()
    }
}

impl fmt::Debug for v2::LocalPairCreateResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalPairCreateResponse")
            .field("ticket", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for v2::LocalPairAcceptRequest {
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

impl fmt::Debug for v2::LocalSessionUnaryRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalSessionUnaryRequest")
            .field("target_device_id", &self.target_device_id)
            .field("frame", &"[REDACTED]")
            .field("frame_len", &self.frame.len())
            .finish()
    }
}

impl fmt::Debug for v2::LocalSessionTunnelOpenRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalSessionTunnelOpenRequest")
            .field("protocol_version", &self.protocol_version)
            .field("target_device_id", &"[REDACTED]")
            .field("target_device_id_present", &self.target_device_id.is_some())
            .finish()
    }
}

impl fmt::Debug for v2::LocalSessionTunnelData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalSessionTunnelData")
            .field("bytes", &"[REDACTED]")
            .field("bytes_len", &self.bytes.len())
            .finish()
    }
}

impl fmt::Debug for v2::LocalStatusResponse {
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

impl fmt::Debug for v2::LocalValidateSetupRequest {
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

impl fmt::Debug for v2::RelayRouteCacheV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayRouteCacheV1")
            .field("format_version", &self.format_version)
            .field("relay_url_count", &self.relay_urls.len())
            .finish()
    }
}

impl fmt::Debug for v2::SessionSummary {
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

impl fmt::Debug for v2::SessionCreateRequest {
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

impl fmt::Debug for v2::ResumeViewId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResumeViewId([REDACTED])")
    }
}

impl fmt::Debug for v2::TerminalAttachRequest {
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

impl fmt::Debug for v2::TerminalInput {
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

/// Product wire major shared by local IPC, `zterm/2`, and `zterm-pair/2`.
pub const WIRE_MAJOR: u32 = zterm_core::WIRE_MAJOR;
/// Current persistent-state schema exposed in readiness/status.
pub const STATE_SCHEMA_VERSION: u32 = zterm_core::STATE_SCHEMA_VERSION;
/// Maximum encoded `WireFrame` body size.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
/// Maximum concrete control-message payload size.
pub const MAX_CONTROL_PAYLOAD_BYTES: usize = 1024 * 1024;
/// Version of the same-UID-only opaque Session tunnel envelope.
pub const LOCAL_SESSION_TUNNEL_VERSION: u32 = 1;
/// Maximum opaque Session bytes carried by one local tunnel Data frame.
pub const MAX_LOCAL_SESSION_TUNNEL_DATA_BYTES: usize = 64 * 1024;
/// Maximum bytes in an unsigned 64-bit varint prefix.
pub const MAX_VARINT_BYTES: usize = 10;

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
    /// Same-UID request for one opaque authenticated Session service stream.
    LocalSessionTunnelOpenRequest = 25,
    /// Confirmation that the remote service stream is ready.
    LocalSessionTunnelOpened = 26,
    /// One bounded opaque chunk of target Session bytes.
    LocalSessionTunnelData = 27,
    /// Address-free path and RTT sideband for this tunnel epoch.
    LocalSessionTunnelPath = 28,
    /// Directional end-of-data marker for one tunnel half.
    LocalSessionTunnelHalfClose = 29,
    /// Content-free terminal outcome for one tunnel epoch.
    LocalSessionTunnelClosed = 30,
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
    /// Complete exact semantic terminal surface.
    TerminalSemanticSnapshot = 301,
    /// Merged exact semantic terminal row replacements.
    TerminalSemanticDelta = 302,
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
    /// Same-UID-only frontend attachment transport-state projection.
    TerminalTransportStateEvent = 311,
    /// Same-UID-only selected connection path and RTT projection.
    TerminalConnectionStatusEvent = 314,
    /// Requests one stateless bounded contiguous history window.
    TerminalHistoryWindowRequest = 317,
    /// Correlated semantic-cell history-window outcome.
    TerminalSemanticHistoryWindowFrame = 318,
    /// Decoded latest-only child clipboard write for the current controller.
    TerminalClipboardWrite = 322,
}

impl WireKind {
    /// Returns whether the kind uses the stricter control-payload limit.
    #[must_use]
    pub const fn is_control(self) -> bool {
        !matches!(
            self,
            Self::TerminalSemanticSnapshot
                | Self::TerminalSemanticDelta
                | Self::TerminalSemanticHistoryWindowFrame
        )
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
        } else if matches!(self, Self::LocalSessionTunnelData) {
            // One field tag plus the three-byte encoded length at this fixed
            // ceiling. This makes a one-byte-oversized canonical Data message
            // fail before it can enter the tunnel adapter.
            MAX_LOCAL_SESSION_TUNNEL_DATA_BYTES + 4
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
            25 => Self::LocalSessionTunnelOpenRequest,
            26 => Self::LocalSessionTunnelOpened,
            27 => Self::LocalSessionTunnelData,
            28 => Self::LocalSessionTunnelPath,
            29 => Self::LocalSessionTunnelHalfClose,
            30 => Self::LocalSessionTunnelClosed,
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
            301 => Self::TerminalSemanticSnapshot,
            302 => Self::TerminalSemanticDelta,
            303 => Self::TerminalInput,
            304 => Self::TerminalResize,
            305 => Self::TerminalDetach,
            306 => Self::TerminalSnapshotApplied,
            307 => Self::TerminalSyncRequest,
            308 => Self::TerminalSyncRequired,
            309 => Self::TerminalLeaseLost,
            310 => Self::TerminalSessionEnded,
            311 => Self::TerminalTransportStateEvent,
            314 => Self::TerminalConnectionStatusEvent,
            317 => Self::TerminalHistoryWindowRequest,
            318 => Self::TerminalSemanticHistoryWindowFrame,
            322 => Self::TerminalClipboardWrite,
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
    /// A semantic surface violated its content-free structural contract.
    InvalidTerminalSurface(TerminalSurfaceError),
    /// A terminal enum or required semantic field used an unsupported value.
    InvalidTerminalSemanticField(&'static str),
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
            Self::InvalidTerminalSurface(error) => error.fmt(formatter),
            Self::InvalidTerminalSemanticField(field) => {
                write!(formatter, "invalid semantic terminal field {field}")
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
    let wire = v2::WireFrame {
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
    let wire = v2::WireFrame::decode(body).map_err(ProtocolError::MalformedProtobuf)?;
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

impl From<DeviceId> for v2::DeviceId {
    fn from(value: DeviceId) -> Self {
        Self {
            value: value.to_bytes().to_vec(),
        }
    }
}

impl TryFrom<v2::DeviceId> for DeviceId {
    type Error = ProtocolError;

    fn try_from(value: v2::DeviceId) -> Result<Self, Self::Error> {
        Self::from_bytes(&value.value).map_err(ProtocolError::InvalidIdentifier)
    }
}

impl From<SessionId> for v2::SessionId {
    fn from(value: SessionId) -> Self {
        Self {
            value: value.to_bytes().to_vec(),
        }
    }
}

impl TryFrom<v2::SessionId> for SessionId {
    type Error = ProtocolError;

    fn try_from(value: v2::SessionId) -> Result<Self, Self::Error> {
        Self::from_bytes(&value.value).map_err(ProtocolError::InvalidIdentifier)
    }
}

impl From<AttachmentId> for v2::AttachmentId {
    fn from(value: AttachmentId) -> Self {
        Self {
            value: value.to_bytes().to_vec(),
        }
    }
}

impl TryFrom<v2::AttachmentId> for AttachmentId {
    type Error = ProtocolError;

    fn try_from(value: v2::AttachmentId) -> Result<Self, Self::Error> {
        Self::from_bytes(&value.value).map_err(ProtocolError::InvalidIdentifier)
    }
}

impl From<ResumeViewId> for v2::ResumeViewId {
    fn from(value: ResumeViewId) -> Self {
        Self {
            value: value.to_bytes().to_vec(),
        }
    }
}

impl TryFrom<v2::ResumeViewId> for ResumeViewId {
    type Error = ProtocolError;

    fn try_from(value: v2::ResumeViewId) -> Result<Self, Self::Error> {
        Self::from_bytes(&value.value).map_err(ProtocolError::InvalidIdentifier)
    }
}

impl From<OperationId> for v2::OperationId {
    fn from(value: OperationId) -> Self {
        Self {
            lease_ordinal: value.lease.ordinal,
            sequence: value.sequence,
            daemon_incarnation: value.lease.daemon_incarnation.to_bytes().to_vec(),
        }
    }
}

impl TryFrom<v2::OperationId> for OperationId {
    type Error = ProtocolError;

    fn try_from(value: v2::OperationId) -> Result<Self, Self::Error> {
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

impl From<OperationLease> for v2::OperationLease {
    fn from(value: OperationLease) -> Self {
        Self {
            daemon_incarnation: value.daemon_incarnation.to_bytes().to_vec(),
            ordinal: value.ordinal,
        }
    }
}

impl TryFrom<v2::OperationLease> for OperationLease {
    type Error = ProtocolError;

    fn try_from(value: v2::OperationLease) -> Result<Self, Self::Error> {
        Ok(Self {
            daemon_incarnation: DaemonIncarnation::from_bytes(&value.daemon_incarnation)
                .map_err(ProtocolError::InvalidIdentifier)?,
            ordinal: value.ordinal,
        })
    }
}

impl From<TerminalSize> for v2::TerminalViewport {
    fn from(value: TerminalSize) -> Self {
        Self {
            rows: u32::from(value.rows),
            columns: u32::from(value.columns),
        }
    }
}

impl TryFrom<v2::TerminalViewport> for TerminalSize {
    type Error = ProtocolError;

    fn try_from(value: v2::TerminalViewport) -> Result<Self, Self::Error> {
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

impl From<ActiveScreen> for v2::TerminalActiveScreen {
    fn from(value: ActiveScreen) -> Self {
        match value {
            ActiveScreen::Main => Self::Main,
            ActiveScreen::Alternate => Self::Alternate,
        }
    }
}

impl From<TerminalModes> for v2::TerminalModes {
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
            alternate_scroll: value.alternate_scroll,
            keyboard_flags: u32::from(value.keyboard_flags.bits()),
        }
    }
}

impl TryFrom<v2::TerminalModes> for TerminalModes {
    type Error = ProtocolError;

    fn try_from(value: v2::TerminalModes) -> Result<Self, Self::Error> {
        let mouse_mode = match value.mouse_mode {
            0 => TerminalMouseMode::None,
            1 => TerminalMouseMode::Press,
            2 => TerminalMouseMode::PressRelease,
            3 => TerminalMouseMode::ButtonMotion,
            4 => TerminalMouseMode::AnyMotion,
            _ => return Err(ProtocolError::InvalidTerminalSemanticField("mouse_mode")),
        };
        let mouse_encoding = match value.mouse_encoding {
            0 => TerminalMouseEncoding::Default,
            1 => TerminalMouseEncoding::Utf8,
            2 => TerminalMouseEncoding::Sgr,
            _ => {
                return Err(ProtocolError::InvalidTerminalSemanticField(
                    "mouse_encoding",
                ));
            }
        };
        let keyboard_flags = u8::try_from(value.keyboard_flags)
            .ok()
            .and_then(TerminalKeyboardFlags::from_bits)
            .ok_or(ProtocolError::InvalidTerminalSemanticField(
                "keyboard_flags",
            ))?;
        Ok(Self {
            application_keypad: value.application_keypad,
            application_cursor: value.application_cursor,
            bracketed_paste: value.bracketed_paste,
            focus_reporting: value.focus_reporting,
            alternate_scroll: value.alternate_scroll,
            mouse_mode,
            mouse_encoding,
            keyboard_flags,
        })
    }
}

/// Projects a validated decoded clipboard write into its redacted wire DTO.
#[must_use]
pub fn terminal_clipboard_write_message(
    attachment_id: AttachmentId,
    write: TerminalClipboardWrite,
) -> v2::TerminalClipboardWrite {
    v2::TerminalClipboardWrite {
        attachment_id: Some(attachment_id.into()),
        text: write.into_string(),
    }
}

/// Validates a decoded clipboard write at the protocol boundary.
pub fn terminal_clipboard_write_from_message(
    value: v2::TerminalClipboardWrite,
) -> Result<(AttachmentId, TerminalClipboardWrite), ProtocolError> {
    let attachment_id = value
        .attachment_id
        .ok_or(ProtocolError::InvalidTerminalSemanticField("attachment_id"))?
        .try_into()?;
    let write = TerminalClipboardWrite::new(value.text)
        .map_err(|_| ProtocolError::InvalidTerminalSemanticField("clipboard_text"))?;
    Ok((attachment_id, write))
}

impl From<TerminalColor> for v2::TerminalColor {
    fn from(value: TerminalColor) -> Self {
        use v2::terminal_color::Value;
        let value = match value {
            TerminalColor::Default => Value::DefaultColor(true),
            TerminalColor::Indexed(index) => Value::Indexed(u32::from(index)),
            TerminalColor::Rgb(red, green, blue) => Value::Rgb(v2::TerminalRgbColor {
                red: u32::from(red),
                green: u32::from(green),
                blue: u32::from(blue),
            }),
        };
        Self { value: Some(value) }
    }
}

impl TryFrom<v2::TerminalColor> for TerminalColor {
    type Error = ProtocolError;

    fn try_from(value: v2::TerminalColor) -> Result<Self, Self::Error> {
        use v2::terminal_color::Value;
        match value.value {
            Some(Value::DefaultColor(true)) => Ok(Self::Default),
            Some(Value::DefaultColor(false)) | None => Err(
                ProtocolError::InvalidTerminalSemanticField("terminal_color"),
            ),
            Some(Value::Indexed(index)) => u8::try_from(index)
                .map(Self::Indexed)
                .map_err(|_| ProtocolError::InvalidTerminalSemanticField("indexed_color")),
            Some(Value::Rgb(rgb)) => Ok(Self::Rgb(
                u8::try_from(rgb.red)
                    .map_err(|_| ProtocolError::InvalidTerminalSemanticField("rgb_red"))?,
                u8::try_from(rgb.green)
                    .map_err(|_| ProtocolError::InvalidTerminalSemanticField("rgb_green"))?,
                u8::try_from(rgb.blue)
                    .map_err(|_| ProtocolError::InvalidTerminalSemanticField("rgb_blue"))?,
            )),
        }
    }
}

impl From<TerminalStyle> for v2::TerminalStyle {
    fn from(value: TerminalStyle) -> Self {
        Self {
            foreground: Some(value.foreground.into()),
            background: Some(value.background.into()),
            bold: value.bold,
            dim: value.dim,
            italic: value.italic,
            underline: value.underline,
            inverse: value.inverse,
        }
    }
}

impl TryFrom<v2::TerminalStyle> for TerminalStyle {
    type Error = ProtocolError;

    fn try_from(value: v2::TerminalStyle) -> Result<Self, Self::Error> {
        Ok(Self {
            foreground: value
                .foreground
                .ok_or(ProtocolError::InvalidTerminalSemanticField("foreground"))?
                .try_into()?,
            background: value
                .background
                .ok_or(ProtocolError::InvalidTerminalSemanticField("background"))?
                .try_into()?,
            bold: value.bold,
            dim: value.dim,
            italic: value.italic,
            underline: value.underline,
            inverse: value.inverse,
        })
    }
}

impl From<TerminalCell> for v2::TerminalCell {
    fn from(value: TerminalCell) -> Self {
        Self {
            contents: value.contents,
            wide: value.wide,
            wide_continuation: value.wide_continuation,
            style: Some(value.style.into()),
        }
    }
}

impl TryFrom<v2::TerminalCell> for TerminalCell {
    type Error = ProtocolError;

    fn try_from(value: v2::TerminalCell) -> Result<Self, Self::Error> {
        Ok(Self {
            contents: value.contents,
            wide: value.wide,
            wide_continuation: value.wide_continuation,
            style: value
                .style
                .ok_or(ProtocolError::InvalidTerminalSemanticField("cell_style"))?
                .try_into()?,
        })
    }
}

impl From<TerminalSurfaceRow> for v2::TerminalSurfaceRow {
    fn from(value: TerminalSurfaceRow) -> Self {
        Self {
            cells: value.cells.into_iter().map(Into::into).collect(),
            wrapped: value.wrapped,
        }
    }
}

impl TryFrom<v2::TerminalSurfaceRow> for TerminalSurfaceRow {
    type Error = ProtocolError;

    fn try_from(value: v2::TerminalSurfaceRow) -> Result<Self, Self::Error> {
        Ok(Self {
            cells: value
                .cells
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            wrapped: value.wrapped,
        })
    }
}

impl From<TerminalCursor> for v2::TerminalCursor {
    fn from(value: TerminalCursor) -> Self {
        Self {
            row: u32::from(value.row),
            column: u32::from(value.column),
            visible: value.visible,
            style: Some(value.style.into()),
        }
    }
}

impl TryFrom<v2::TerminalCursor> for TerminalCursor {
    type Error = ProtocolError;

    fn try_from(value: v2::TerminalCursor) -> Result<Self, Self::Error> {
        Ok(Self {
            row: u16::try_from(value.row)
                .map_err(|_| ProtocolError::InvalidTerminalSemanticField("cursor_row"))?,
            column: u16::try_from(value.column)
                .map_err(|_| ProtocolError::InvalidTerminalSemanticField("cursor_column"))?,
            visible: value.visible,
            style: value
                .style
                .ok_or(ProtocolError::InvalidTerminalSemanticField("cursor_style"))?
                .try_into()?,
        })
    }
}

impl From<TerminalScrollMetrics> for v2::TerminalScrollMetrics {
    fn from(value: TerminalScrollMetrics) -> Self {
        Self {
            epoch: value.epoch.get(),
            revision: value.revision.get(),
            offset_from_bottom: value.offset_from_bottom,
            max_offset_from_bottom: value.max_offset_from_bottom,
            viewport_rows: u32::from(value.viewport_rows),
        }
    }
}

impl From<TerminalHistoryWindowAnchor> for v2::TerminalHistoryWindowAnchor {
    fn from(value: TerminalHistoryWindowAnchor) -> Self {
        Self {
            epoch: value.epoch.get(),
            revision: value.revision.get(),
            max_offset_from_bottom: value.max_offset_from_bottom,
            viewport_rows: u32::from(value.viewport.rows),
            viewport_columns: u32::from(value.viewport.columns),
        }
    }
}

fn terminal_active_screen(value: i32) -> Result<ActiveScreen, ProtocolError> {
    match v2::TerminalActiveScreen::try_from(value).ok() {
        Some(v2::TerminalActiveScreen::Main) => Ok(ActiveScreen::Main),
        Some(v2::TerminalActiveScreen::Alternate) => Ok(ActiveScreen::Alternate),
        Some(v2::TerminalActiveScreen::Unspecified) | None => {
            Err(ProtocolError::InvalidTerminalSemanticField("active_screen"))
        }
    }
}

fn terminal_scroll_metrics(
    value: v2::TerminalScrollMetrics,
) -> Result<TerminalScrollMetrics, ProtocolError> {
    Ok(TerminalScrollMetrics {
        epoch: zterm_core::Revision::new(value.epoch),
        revision: zterm_core::Revision::new(value.revision),
        offset_from_bottom: value.offset_from_bottom,
        max_offset_from_bottom: value.max_offset_from_bottom,
        viewport_rows: u16::try_from(value.viewport_rows).map_err(|_| {
            ProtocolError::InvalidTerminalSemanticField("scroll_metrics_viewport_rows")
        })?,
    })
}

fn terminal_surface_message(surface: TerminalSurface) -> v2::TerminalSurface {
    v2::TerminalSurface {
        row_count: u32::from(surface.size.rows),
        column_count: u32::from(surface.size.columns),
        active_screen: v2::TerminalActiveScreen::from(surface.active_screen) as i32,
        rows: surface.rows.into_iter().map(Into::into).collect(),
        cursor: Some(surface.cursor.into()),
        modes: Some(surface.modes.into()),
        scroll_metrics: surface.scroll_metrics.map(Into::into),
    }
}

fn terminal_surface_from_message(
    value: v2::TerminalSurface,
    revision: zterm_core::Revision,
) -> Result<TerminalSurface, ProtocolError> {
    let size = TerminalSize::try_from(v2::TerminalViewport {
        rows: value.row_count,
        columns: value.column_count,
    })?;
    let surface = TerminalSurface {
        size,
        active_screen: terminal_active_screen(value.active_screen)?,
        rows: value
            .rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?,
        cursor: value
            .cursor
            .ok_or(ProtocolError::InvalidTerminalSemanticField("cursor"))?
            .try_into()?,
        modes: value
            .modes
            .ok_or(ProtocolError::InvalidTerminalSemanticField("modes"))?
            .try_into()?,
        scroll_metrics: value
            .scroll_metrics
            .map(terminal_scroll_metrics)
            .transpose()?,
    };
    surface
        .validate(revision)
        .map_err(ProtocolError::InvalidTerminalSurface)?;
    Ok(surface)
}

/// Projects one exact semantic snapshot into its content-redacted wire message.
#[must_use]
pub fn terminal_surface_snapshot_message(
    session_id: SessionId,
    attachment_id: AttachmentId,
    snapshot: TerminalSurfaceSnapshot,
) -> v2::TerminalSemanticSnapshot {
    v2::TerminalSemanticSnapshot {
        session_id: Some(session_id.into()),
        attachment_id: Some(attachment_id.into()),
        revision: snapshot.revision.get(),
        surface: Some(terminal_surface_message(snapshot.surface)),
    }
}

/// Validates and converts one exact semantic snapshot wire message.
pub fn terminal_surface_snapshot_from_message(
    value: v2::TerminalSemanticSnapshot,
) -> Result<(SessionId, AttachmentId, TerminalSurfaceSnapshot), ProtocolError> {
    let session_id = value
        .session_id
        .ok_or(ProtocolError::InvalidTerminalSemanticField("session_id"))?
        .try_into()?;
    let attachment_id = value
        .attachment_id
        .ok_or(ProtocolError::InvalidTerminalSemanticField("attachment_id"))?
        .try_into()?;
    let revision = zterm_core::Revision::new(value.revision);
    let surface = terminal_surface_from_message(
        value
            .surface
            .ok_or(ProtocolError::InvalidTerminalSemanticField("surface"))?,
        revision,
    )?;
    Ok((
        session_id,
        attachment_id,
        TerminalSurfaceSnapshot { revision, surface },
    ))
}

/// Projects one exact semantic delta into its content-redacted wire message.
#[must_use]
pub fn terminal_surface_delta_message(
    attachment_id: AttachmentId,
    delta: TerminalSurfaceDelta,
) -> v2::TerminalSemanticDelta {
    v2::TerminalSemanticDelta {
        attachment_id: Some(attachment_id.into()),
        from_revision: delta.from_revision.get(),
        to_revision: delta.to_revision.get(),
        row_count: u32::from(delta.size.rows),
        column_count: u32::from(delta.size.columns),
        active_screen: v2::TerminalActiveScreen::from(delta.active_screen) as i32,
        row_patches: delta
            .row_patches
            .into_iter()
            .map(|patch| v2::TerminalSemanticRowPatch {
                row: u32::from(patch.row),
                replacement: Some(patch.replacement.into()),
            })
            .collect(),
        cursor: Some(delta.cursor.into()),
        modes: Some(delta.modes.into()),
        scroll_metrics: delta.scroll_metrics.map(Into::into),
    }
}

/// Validates and converts one exact semantic delta wire message.
pub fn terminal_surface_delta_from_message(
    value: v2::TerminalSemanticDelta,
) -> Result<(AttachmentId, TerminalSurfaceDelta), ProtocolError> {
    let attachment_id = value
        .attachment_id
        .ok_or(ProtocolError::InvalidTerminalSemanticField("attachment_id"))?
        .try_into()?;
    let delta = TerminalSurfaceDelta {
        from_revision: zterm_core::Revision::new(value.from_revision),
        to_revision: zterm_core::Revision::new(value.to_revision),
        size: TerminalSize::try_from(v2::TerminalViewport {
            rows: value.row_count,
            columns: value.column_count,
        })?,
        active_screen: terminal_active_screen(value.active_screen)?,
        row_patches: value
            .row_patches
            .into_iter()
            .map(|patch| {
                Ok(TerminalSurfaceRowPatch {
                    row: u16::try_from(patch.row).map_err(|_| {
                        ProtocolError::InvalidTerminalSemanticField("row_patch_index")
                    })?,
                    replacement: patch
                        .replacement
                        .ok_or(ProtocolError::InvalidTerminalSemanticField(
                            "row_patch_replacement",
                        ))?
                        .try_into()?,
                })
            })
            .collect::<Result<_, ProtocolError>>()?,
        cursor: value
            .cursor
            .ok_or(ProtocolError::InvalidTerminalSemanticField("cursor"))?
            .try_into()?,
        modes: value
            .modes
            .ok_or(ProtocolError::InvalidTerminalSemanticField("modes"))?
            .try_into()?,
        scroll_metrics: value
            .scroll_metrics
            .map(terminal_scroll_metrics)
            .transpose()?,
    };
    delta
        .validate()
        .map_err(ProtocolError::InvalidTerminalSurface)?;
    Ok((attachment_id, delta))
}

/// Projects one semantic history-window result into a content-redacted wire message.
#[must_use]
pub fn terminal_surface_history_window_frame_message(
    attachment_id: AttachmentId,
    result: TerminalSurfaceHistoryWindowResult,
) -> v2::TerminalSemanticHistoryWindowFrame {
    match result {
        TerminalSurfaceHistoryWindowResult::Frame(TerminalSurfaceHistoryWindowFrame {
            disposition,
            anchor,
            target_offset_from_bottom,
            first_row_from_live_top,
            rows,
        }) => v2::TerminalSemanticHistoryWindowFrame {
            attachment_id: Some(attachment_id.into()),
            outcome: v2::TerminalHistoryWindowOutcome::Frame as i32,
            disposition: match disposition {
                TerminalViewportDisposition::Exact => v2::TerminalViewportDisposition::Exact as i32,
                TerminalViewportDisposition::Rebased => {
                    v2::TerminalViewportDisposition::Rebased as i32
                }
            },
            anchor: Some(anchor.into()),
            target_offset_from_bottom,
            first_row_from_live_top,
            rows: rows.into_iter().map(Into::into).collect(),
            current_epoch: anchor.epoch.get(),
            current_revision: anchor.revision.get(),
        },
        TerminalSurfaceHistoryWindowResult::HistoryChanged { epoch, revision } => {
            v2::TerminalSemanticHistoryWindowFrame {
                attachment_id: Some(attachment_id.into()),
                outcome: v2::TerminalHistoryWindowOutcome::Changed as i32,
                disposition: v2::TerminalViewportDisposition::Unspecified as i32,
                anchor: None,
                target_offset_from_bottom: 0,
                first_row_from_live_top: 0,
                rows: Vec::new(),
                current_epoch: epoch.get(),
                current_revision: revision.get(),
            }
        }
        TerminalSurfaceHistoryWindowResult::HistoryGap { epoch, revision } => {
            v2::TerminalSemanticHistoryWindowFrame {
                attachment_id: Some(attachment_id.into()),
                outcome: v2::TerminalHistoryWindowOutcome::Gap as i32,
                disposition: v2::TerminalViewportDisposition::Unspecified as i32,
                anchor: None,
                target_offset_from_bottom: 0,
                first_row_from_live_top: 0,
                rows: Vec::new(),
                current_epoch: epoch.get(),
                current_revision: revision.get(),
            }
        }
    }
}

/// Validates and converts one request-bound semantic history-window response.
pub fn terminal_surface_history_window_from_message(
    value: v2::TerminalSemanticHistoryWindowFrame,
    query: TerminalHistoryWindowQuery,
) -> Result<(AttachmentId, TerminalSurfaceHistoryWindowResult), ProtocolError> {
    let attachment_id = value
        .attachment_id
        .ok_or(ProtocolError::InvalidTerminalSemanticField("attachment_id"))?
        .try_into()?;
    let outcome = v2::TerminalHistoryWindowOutcome::try_from(value.outcome)
        .map_err(|_| ProtocolError::InvalidTerminalSemanticField("history_window_outcome"))?;
    let result = match outcome {
        v2::TerminalHistoryWindowOutcome::Frame => {
            let disposition = match v2::TerminalViewportDisposition::try_from(value.disposition)
                .ok()
            {
                Some(v2::TerminalViewportDisposition::Exact) => TerminalViewportDisposition::Exact,
                Some(v2::TerminalViewportDisposition::Rebased) => {
                    TerminalViewportDisposition::Rebased
                }
                _ => {
                    return Err(ProtocolError::InvalidTerminalSemanticField(
                        "history_window_disposition",
                    ));
                }
            };
            let anchor = value
                .anchor
                .ok_or(ProtocolError::InvalidTerminalSemanticField(
                    "history_window_anchor",
                ))?;
            let anchor = TerminalHistoryWindowAnchor {
                epoch: zterm_core::Revision::new(anchor.epoch),
                revision: zterm_core::Revision::new(anchor.revision),
                max_offset_from_bottom: anchor.max_offset_from_bottom,
                viewport: TerminalSize::try_from(v2::TerminalViewport {
                    rows: anchor.viewport_rows,
                    columns: anchor.viewport_columns,
                })?,
            };
            if value.current_epoch != anchor.epoch.get()
                || value.current_revision != anchor.revision.get()
            {
                return Err(ProtocolError::InvalidTerminalSemanticField(
                    "history_window_current_revision",
                ));
            }
            let frame = TerminalSurfaceHistoryWindowFrame {
                disposition,
                anchor,
                target_offset_from_bottom: value.target_offset_from_bottom,
                first_row_from_live_top: value.first_row_from_live_top,
                rows: value
                    .rows
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_, _>>()?,
            };
            frame
                .validate_for(query)
                .map_err(ProtocolError::InvalidTerminalSurface)?;
            TerminalSurfaceHistoryWindowResult::Frame(frame)
        }
        v2::TerminalHistoryWindowOutcome::Changed | v2::TerminalHistoryWindowOutcome::Gap => {
            if value.disposition != v2::TerminalViewportDisposition::Unspecified as i32
                || value.anchor.is_some()
                || value.target_offset_from_bottom != 0
                || value.first_row_from_live_top != 0
                || !value.rows.is_empty()
                || value.current_epoch > value.current_revision
                || value.current_revision < query.anchor.revision.get()
            {
                return Err(ProtocolError::InvalidTerminalSurface(
                    TerminalSurfaceError::InvalidHistoryWindow,
                ));
            }
            let fields = {
                let epoch = zterm_core::Revision::new(value.current_epoch);
                let revision = zterm_core::Revision::new(value.current_revision);
                (epoch, revision)
            };
            if outcome == v2::TerminalHistoryWindowOutcome::Changed {
                TerminalSurfaceHistoryWindowResult::HistoryChanged {
                    epoch: fields.0,
                    revision: fields.1,
                }
            } else {
                TerminalSurfaceHistoryWindowResult::HistoryGap {
                    epoch: fields.0,
                    revision: fields.1,
                }
            }
        }
        v2::TerminalHistoryWindowOutcome::Unspecified => {
            return Err(ProtocolError::InvalidTerminalSemanticField(
                "history_window_outcome",
            ));
        }
    };
    Ok((attachment_id, result))
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

impl From<(&PairTicketFields, &PairSecret)> for v2::PairTicketV1 {
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

impl TryFrom<v2::PairTicketV1> for (PairTicketFields, PairSecret) {
    type Error = TicketTextError;

    fn try_from(value: v2::PairTicketV1) -> Result<Self, Self::Error> {
        let v2::PairTicketV1 {
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
    let mut message = v2::PairTicketV1::from((fields, secret));
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
        v2::PairTicketV1::decode(decoded.as_slice()).map_err(TicketTextError::InvalidProtobuf)?;
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
    Ok(v2::RelayRouteCacheV1 {
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
    let cache = v2::RelayRouteCacheV1::decode(bytes).map_err(RouteCacheError::Malformed)?;
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

impl TryFrom<v2::PairBegin> for PairBegin {
    type Error = WireFieldError;

    fn try_from(value: v2::PairBegin) -> Result<Self, Self::Error> {
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

impl From<&PairBegin> for v2::PairBegin {
    fn from(value: &PairBegin) -> Self {
        Self {
            offer_id: value.offer_id().to_bytes().to_vec(),
            controller_name: value.controller_name().to_owned(),
            controller_nonce: value.controller_nonce().to_bytes().to_vec(),
            pair_protocol_version: value.pair_protocol_version(),
        }
    }
}

impl TryFrom<v2::PairChallenge> for PairChallenge {
    type Error = WireFieldError;

    fn try_from(value: v2::PairChallenge) -> Result<Self, Self::Error> {
        let host_nonce =
            PairNonce::from_bytes(&value.host_nonce).map_err(WireFieldError::InvalidIdentifier)?;
        PairChallenge::new(host_nonce, value.selected_version, value.ticket_expiry_unix)
            .map_err(WireFieldError::InvalidPair)
    }
}

impl From<&PairChallenge> for v2::PairChallenge {
    fn from(value: &PairChallenge) -> Self {
        Self {
            host_nonce: value.host_nonce().to_bytes().to_vec(),
            selected_version: value.selected_version(),
            ticket_expiry_unix: value.ticket_expiry_unix(),
        }
    }
}

impl TryFrom<v2::PairProof> for PairProof {
    type Error = WireFieldError;

    fn try_from(value: v2::PairProof) -> Result<Self, Self::Error> {
        let proof = Zeroizing::new(value.controller_proof);
        PairProof::from_slice(&proof).map_err(WireFieldError::InvalidIdentifier)
    }
}

impl From<&PairProof> for v2::PairProof {
    fn from(value: &PairProof) -> Self {
        Self {
            controller_proof: value.as_bytes().to_vec(),
        }
    }
}

impl TryFrom<v2::PairAccepted> for PairAccepted {
    type Error = WireFieldError;

    fn try_from(value: v2::PairAccepted) -> Result<Self, Self::Error> {
        let v2::PairAccepted {
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

impl From<&PairAccepted> for v2::PairAccepted {
    fn from(value: &PairAccepted) -> Self {
        Self {
            authorization_generation: value.authorization_generation().get(),
            host_confirmation_proof: value.host_confirmation_proof().to_vec(),
            host_diagnostic_version: value.host_diagnostic_version().to_owned(),
        }
    }
}

impl TryFrom<v2::ConnectionHello> for ConnectionHello {
    type Error = WireFieldError;

    fn try_from(value: v2::ConnectionHello) -> Result<Self, Self::Error> {
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

impl From<&ConnectionHello> for v2::ConnectionHello {
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

impl TryFrom<v2::ConnectionWelcome> for ConnectionWelcome {
    type Error = WireFieldError;

    fn try_from(value: v2::ConnectionWelcome) -> Result<Self, Self::Error> {
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

impl From<&ConnectionWelcome> for v2::ConnectionWelcome {
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

impl TryFrom<v2::LocalDeviceRenameRequest> for (DeviceId, DeviceAlias) {
    type Error = WireFieldError;

    fn try_from(value: v2::LocalDeviceRenameRequest) -> Result<Self, Self::Error> {
        let device_bytes = value.device_id.unwrap_or_default().value;
        let device_id =
            DeviceId::from_bytes(&device_bytes).map_err(WireFieldError::InvalidIdentifier)?;
        let alias = DeviceAlias::new(value.alias).map_err(WireFieldError::InvalidAlias)?;
        Ok((device_id, alias))
    }
}

impl TryFrom<v2::LocalDeviceRevokeRequest> for DeviceId {
    type Error = WireFieldError;

    fn try_from(value: v2::LocalDeviceRevokeRequest) -> Result<Self, Self::Error> {
        let device_bytes = value.device_id.unwrap_or_default().value;
        DeviceId::from_bytes(&device_bytes).map_err(WireFieldError::InvalidIdentifier)
    }
}

impl TryFrom<v2::DeviceSummary> for DeviceSummary {
    type Error = WireFieldError;

    fn try_from(value: v2::DeviceSummary) -> Result<Self, Self::Error> {
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
        let auth_status = match v2::DeviceAuthStatus::try_from(value.auth_status) {
            Ok(v2::DeviceAuthStatus::None) => AuthorizationStatus::None,
            Ok(v2::DeviceAuthStatus::Authorized) => AuthorizationStatus::Authorized,
            Ok(v2::DeviceAuthStatus::Revoked) => AuthorizationStatus::Revoked,
            Ok(v2::DeviceAuthStatus::Unspecified) | Err(_) => {
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

impl From<&DeviceSummary> for v2::DeviceSummary {
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
                AuthorizationStatus::None => v2::DeviceAuthStatus::None as i32,
                AuthorizationStatus::Authorized => v2::DeviceAuthStatus::Authorized as i32,
                AuthorizationStatus::Revoked => v2::DeviceAuthStatus::Revoked as i32,
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
    use zterm_core::terminal::MAX_TERMINAL_CLIPBOARD_BYTES;

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
        let message = v2::LocalStatusRequest {};
        let mut bytes = encode_message(WireKind::LocalStatusRequest, 17, 5_000, &message)
            .expect("bounded status frame");
        assert_eq!(
            bytes,
            [0x09, 0x08, 0x02, 0x10, 0x03, 0x20, 0x11, 0x28, 0x88, 0x27],
            "language-neutral v2 empty-status golden frame"
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
        let decoded: v2::LocalStatusRequest = frames[0]
            .decode_message(WireKind::LocalStatusRequest)
            .expect("typed payload");
        assert_eq!(decoded, message);
        assert_eq!(frames[0].request_id, 17);
    }

    #[test]
    fn local_session_tunnel_envelope_is_bounded_and_redacted() {
        const SECRET: &[u8] = b"TUNNEL_PAYLOAD_SECRET_22e8";
        let target = DeviceId::from_array([0x7a; DeviceId::LENGTH]);
        let open = v2::LocalSessionTunnelOpenRequest {
            protocol_version: LOCAL_SESSION_TUNNEL_VERSION,
            target_device_id: Some(target.into()),
        };
        assert_message_round_trip(WireKind::LocalSessionTunnelOpenRequest, open.clone());
        let open_debug = format!("{open:?}");
        assert!(!open_debug.contains(&target.to_string()));

        let maximum = v2::LocalSessionTunnelData {
            bytes: vec![0x5a; MAX_LOCAL_SESSION_TUNNEL_DATA_BYTES],
        };
        assert_message_round_trip(WireKind::LocalSessionTunnelData, maximum);
        let oversized = v2::LocalSessionTunnelData {
            bytes: vec![0x5a; MAX_LOCAL_SESSION_TUNNEL_DATA_BYTES + 1],
        };
        assert!(matches!(
            encode_message(WireKind::LocalSessionTunnelData, 0, 0, &oversized),
            Err(ProtocolError::ControlPayloadTooLarge(_))
        ));

        let secret = v2::LocalSessionTunnelData {
            bytes: SECRET.to_vec(),
        };
        let debug = format!("{secret:?}");
        assert!(debug.contains(&format!("bytes_len: {}", SECRET.len())));
        assert!(!debug.contains(std::str::from_utf8(SECRET).expect("ASCII secret")));
    }

    #[test]
    fn decoder_rejects_unknown_major_kind_and_incomplete_or_malformed_lengths() {
        let unknown_major = v2::WireFrame {
            wire_major: 1,
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
            Err(ProtocolError::WireMajorMismatch { actual: 1, .. })
        ));

        let unknown_kind = v2::WireFrame {
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
        let message = v2::LocalSessionUnaryRequest {
            target_device_id: Some(DeviceId::from_array([7; 32]).into()),
            frame: SENTINEL.to_vec(),
        };
        let debug = format!("{message:?}");
        assert!(!debug.contains("REMOTE-SESSION-INNER-FRAME-SENTINEL"));
        assert!(debug.contains("frame_len"));
        assert_message_round_trip(WireKind::LocalSessionUnaryRequest, message);

        let bounded_tunnel = v2::LocalSessionUnaryRequest {
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
        let oversized_control = v2::WireFrame {
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
            WireKind::TerminalSemanticSnapshot,
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
    fn fixed_width_domain_ids_validate_once_at_proto_boundary() {
        let device = DeviceId::from_array([42; 32]);
        let decoded = DeviceId::try_from(v2::DeviceId::from(device)).expect("valid device id");
        assert_eq!(decoded, device);
        assert!(matches!(
            DeviceId::try_from(v2::DeviceId { value: vec![0; 31] }),
            Err(ProtocolError::InvalidIdentifier(_))
        ));
    }

    #[test]
    fn wire_kind_registry_matches_the_proto_source_of_truth() {
        let kinds = [
            (
                WireKind::LocalReadinessRequest,
                v2::MessageKind::LocalReadinessRequest as u32,
            ),
            (
                WireKind::LocalReadinessResponse,
                v2::MessageKind::LocalReadinessResponse as u32,
            ),
            (
                WireKind::LocalStatusRequest,
                v2::MessageKind::LocalStatusRequest as u32,
            ),
            (
                WireKind::LocalStatusResponse,
                v2::MessageKind::LocalStatusResponse as u32,
            ),
            (
                WireKind::LocalValidateSetupRequest,
                v2::MessageKind::LocalValidateSetupRequest as u32,
            ),
            (
                WireKind::LocalValidateSetupResponse,
                v2::MessageKind::LocalValidateSetupResponse as u32,
            ),
            (
                WireKind::LocalStopRequest,
                v2::MessageKind::LocalStopRequest as u32,
            ),
            (
                WireKind::LocalStopResponse,
                v2::MessageKind::LocalStopResponse as u32,
            ),
            (
                WireKind::LocalUpdatePreflightRequest,
                v2::MessageKind::LocalUpdatePreflightRequest as u32,
            ),
            (
                WireKind::LocalUpdatePreflightResponse,
                v2::MessageKind::LocalUpdatePreflightResponse as u32,
            ),
            (
                WireKind::ServiceErrorResponse,
                v2::MessageKind::ServiceErrorResponse as u32,
            ),
            (
                WireKind::LocalPairCreateRequest,
                v2::MessageKind::LocalPairCreateRequest as u32,
            ),
            (
                WireKind::LocalPairCreateResponse,
                v2::MessageKind::LocalPairCreateResponse as u32,
            ),
            (
                WireKind::LocalPairAcceptRequest,
                v2::MessageKind::LocalPairAcceptRequest as u32,
            ),
            (
                WireKind::LocalPairAcceptResponse,
                v2::MessageKind::LocalPairAcceptResponse as u32,
            ),
            (
                WireKind::LocalDeviceListRequest,
                v2::MessageKind::LocalDeviceListRequest as u32,
            ),
            (
                WireKind::LocalDeviceListResponse,
                v2::MessageKind::LocalDeviceListResponse as u32,
            ),
            (
                WireKind::LocalDeviceRenameRequest,
                v2::MessageKind::LocalDeviceRenameRequest as u32,
            ),
            (
                WireKind::LocalDeviceRenameResponse,
                v2::MessageKind::LocalDeviceRenameResponse as u32,
            ),
            (
                WireKind::LocalDeviceRevokeRequest,
                v2::MessageKind::LocalDeviceRevokeRequest as u32,
            ),
            (
                WireKind::LocalDeviceRevokeResponse,
                v2::MessageKind::LocalDeviceRevokeResponse as u32,
            ),
            (
                WireKind::LocalTargetResolveRequest,
                v2::MessageKind::LocalTargetResolveRequest as u32,
            ),
            (
                WireKind::LocalTargetResolveResponse,
                v2::MessageKind::LocalTargetResolveResponse as u32,
            ),
            (
                WireKind::LocalSessionUnaryRequest,
                v2::MessageKind::LocalSessionUnaryRequest as u32,
            ),
            (
                WireKind::LocalSessionTunnelOpenRequest,
                v2::MessageKind::LocalSessionTunnelOpenRequest as u32,
            ),
            (
                WireKind::LocalSessionTunnelOpened,
                v2::MessageKind::LocalSessionTunnelOpened as u32,
            ),
            (
                WireKind::LocalSessionTunnelData,
                v2::MessageKind::LocalSessionTunnelData as u32,
            ),
            (
                WireKind::LocalSessionTunnelPath,
                v2::MessageKind::LocalSessionTunnelPath as u32,
            ),
            (
                WireKind::LocalSessionTunnelHalfClose,
                v2::MessageKind::LocalSessionTunnelHalfClose as u32,
            ),
            (
                WireKind::LocalSessionTunnelClosed,
                v2::MessageKind::LocalSessionTunnelClosed as u32,
            ),
            (WireKind::PairBegin, v2::MessageKind::PairBegin as u32),
            (
                WireKind::PairChallenge,
                v2::MessageKind::PairChallenge as u32,
            ),
            (WireKind::PairProof, v2::MessageKind::PairProof as u32),
            (WireKind::PairAccepted, v2::MessageKind::PairAccepted as u32),
            (
                WireKind::ConnectionHello,
                v2::MessageKind::ConnectionHello as u32,
            ),
            (
                WireKind::ConnectionWelcome,
                v2::MessageKind::ConnectionWelcome as u32,
            ),
            (
                WireKind::SessionListRequest,
                v2::MessageKind::SessionListRequest as u32,
            ),
            (
                WireKind::SessionListResponse,
                v2::MessageKind::SessionListResponse as u32,
            ),
            (
                WireKind::SessionCreateRequest,
                v2::MessageKind::SessionCreateRequest as u32,
            ),
            (
                WireKind::SessionMutateResponse,
                v2::MessageKind::SessionMutateResponse as u32,
            ),
            (
                WireKind::SessionRenameRequest,
                v2::MessageKind::SessionRenameRequest as u32,
            ),
            (
                WireKind::SessionCloseRequest,
                v2::MessageKind::SessionCloseRequest as u32,
            ),
            (
                WireKind::SessionTakeoverRequest,
                v2::MessageKind::SessionTakeoverRequest as u32,
            ),
            (
                WireKind::SessionOperationLeaseRequest,
                v2::MessageKind::SessionOperationLeaseRequest as u32,
            ),
            (
                WireKind::SessionOperationLeaseResponse,
                v2::MessageKind::SessionOperationLeaseResponse as u32,
            ),
            (
                WireKind::TerminalAttachRequest,
                v2::MessageKind::TerminalAttachRequest as u32,
            ),
            (
                WireKind::TerminalInput,
                v2::MessageKind::TerminalInput as u32,
            ),
            (
                WireKind::TerminalResize,
                v2::MessageKind::TerminalResize as u32,
            ),
            (
                WireKind::TerminalDetach,
                v2::MessageKind::TerminalDetach as u32,
            ),
            (
                WireKind::TerminalSnapshotApplied,
                v2::MessageKind::TerminalSnapshotApplied as u32,
            ),
            (
                WireKind::TerminalSyncRequest,
                v2::MessageKind::TerminalSyncRequest as u32,
            ),
            (
                WireKind::TerminalSyncRequired,
                v2::MessageKind::TerminalSyncRequired as u32,
            ),
            (
                WireKind::TerminalLeaseLost,
                v2::MessageKind::TerminalLeaseLost as u32,
            ),
            (
                WireKind::TerminalSessionEnded,
                v2::MessageKind::TerminalSessionEnded as u32,
            ),
            (
                WireKind::TerminalTransportStateEvent,
                v2::MessageKind::TerminalTransportStateEvent as u32,
            ),
            (
                WireKind::TerminalConnectionStatusEvent,
                v2::MessageKind::TerminalConnectionStatusEvent as u32,
            ),
            (
                WireKind::TerminalHistoryWindowRequest,
                v2::MessageKind::TerminalHistoryWindowRequest as u32,
            ),
            (
                WireKind::TerminalSemanticSnapshot,
                v2::MessageKind::TerminalSemanticSnapshot as u32,
            ),
            (
                WireKind::TerminalSemanticDelta,
                v2::MessageKind::TerminalSemanticDelta as u32,
            ),
            (
                WireKind::TerminalSemanticHistoryWindowFrame,
                v2::MessageKind::TerminalSemanticHistoryWindowFrame as u32,
            ),
            (
                WireKind::TerminalClipboardWrite,
                v2::MessageKind::TerminalClipboardWrite as u32,
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
        let operation_id = Some(v2::OperationId {
            lease_ordinal: 7,
            sequence: 11,
            daemon_incarnation: vec![9; 16],
        });
        let target = Some(v2::TargetSelector {
            target: Some(v2::target_selector::Target::Device(v2::DeviceId {
                value: vec![7; DeviceId::LENGTH],
            })),
        });
        let session_id = Some(v2::SessionId { value: vec![3; 16] });
        let attachment_id = Some(v2::AttachmentId { value: vec![5; 16] });

        assert_message_round_trip(
            WireKind::SessionRenameRequest,
            v2::SessionRenameRequest {
                operation_id: operation_id.clone(),
                target: target.clone(),
                session_id: session_id.clone(),
                name: "build".to_owned(),
            },
        );
        assert_message_round_trip(
            WireKind::SessionCloseRequest,
            v2::SessionCloseRequest {
                operation_id: operation_id.clone(),
                target: target.clone(),
                session_id: session_id.clone(),
            },
        );
        assert_message_round_trip(
            WireKind::TerminalAttachRequest,
            v2::TerminalAttachRequest {
                target: target.clone(),
                session_id: session_id.clone(),
                takeover: true,
                session_name: String::new(),
                create_main: false,
                viewport: Some(v2::TerminalViewport {
                    rows: 40,
                    columns: 120,
                }),
                resume_view_id: Some(v2::ResumeViewId { value: vec![6; 16] }),
                known_revision: Some(12),
            },
        );
        assert_message_round_trip(
            WireKind::SessionTakeoverRequest,
            v2::SessionTakeoverRequest {
                operation_id,
                target,
                session_id,
                attachment_id: attachment_id.clone(),
            },
        );
        assert_message_round_trip(
            WireKind::TerminalSnapshotApplied,
            v2::TerminalSnapshotApplied {
                attachment_id: attachment_id.clone(),
                revision: 13,
            },
        );
        assert_message_round_trip(
            WireKind::TerminalSyncRequest,
            v2::TerminalSyncRequest {
                attachment_id: attachment_id.clone(),
                known_revision: 13,
            },
        );
        assert_message_round_trip(
            WireKind::TerminalSyncRequired,
            v2::TerminalSyncRequired {
                attachment_id: attachment_id.clone(),
                latest_revision: 17,
            },
        );
        assert_message_round_trip(
            WireKind::TerminalLeaseLost,
            v2::TerminalLeaseLost {
                attachment_id: Some(v2::AttachmentId { value: vec![5; 16] }),
                generation: 3,
            },
        );
        assert_message_round_trip(
            WireKind::TerminalSessionEnded,
            v2::TerminalSessionEnded {
                session_id: Some(v2::SessionId { value: vec![3; 16] }),
                attachment_id: Some(v2::AttachmentId { value: vec![5; 16] }),
                reason: v2::TerminalSessionEndReason::NaturalExit as i32,
                exit_code: 0,
                signal: String::new(),
            },
        );
        assert_message_round_trip(
            WireKind::TerminalTransportStateEvent,
            v2::TerminalTransportStateEvent {
                attachment_id: Some(v2::AttachmentId { value: vec![5; 16] }),
                state: v2::TerminalTransportState::Reconnecting as i32,
            },
        );
        assert_message_round_trip(
            WireKind::TerminalConnectionStatusEvent,
            v2::TerminalConnectionStatusEvent {
                attachment_id: attachment_id.clone(),
                path: v2::TerminalConnectionPath::Direct as i32,
                rtt_ms: Some(42),
            },
        );
        let window_anchor = v2::TerminalHistoryWindowAnchor {
            epoch: 3,
            revision: 17,
            max_offset_from_bottom: 9,
            viewport_rows: 2,
            viewport_columns: 80,
        };
        assert_message_round_trip(
            WireKind::TerminalHistoryWindowRequest,
            v2::TerminalHistoryWindowRequest {
                attachment_id: attachment_id.clone(),
                anchor: Some(window_anchor),
                target_offset_from_bottom: 3,
                older_margin_rows: 2,
                newer_margin_rows: 2,
            },
        );
        assert_eq!(
            ResumeViewId::try_from(v2::ResumeViewId { value: vec![6; 16] })
                .expect("fixed-width resume view ID"),
            ResumeViewId::from_array([6; 16])
        );
        assert!(matches!(
            ResumeViewId::try_from(v2::ResumeViewId { value: vec![6; 15] }),
            Err(ProtocolError::InvalidIdentifier(_))
        ));
        assert_eq!(
            TerminalSize::try_from(v2::TerminalViewport {
                rows: 40,
                columns: 120,
            })
            .expect("bounded viewport"),
            TerminalSize::new(40, 120)
        );
        assert!(matches!(
            TerminalSize::try_from(v2::TerminalViewport {
                rows: 0,
                columns: 120,
            }),
            Err(ProtocolError::InvalidTerminalSize { .. })
        ));
    }

    fn semantic_row(columns: u16, contents: &str) -> TerminalSurfaceRow {
        TerminalSurfaceRow {
            cells: (0..columns)
                .map(|_| TerminalCell {
                    contents: contents.to_owned(),
                    style: TerminalStyle {
                        foreground: TerminalColor::Rgb(1, 2, 3),
                        background: TerminalColor::Rgb(4, 5, 6),
                        bold: true,
                        dim: true,
                        italic: true,
                        underline: true,
                        inverse: true,
                    },
                    ..TerminalCell::default()
                })
                .collect(),
            wrapped: true,
        }
    }

    #[test]
    fn semantic_surface_messages_round_trip_validate_and_redact_content() {
        const SENTINEL: &str = "SEMANTIC_57c1";
        let session_id = SessionId::from_array([3; 16]);
        let attachment_id = AttachmentId::from_array([4; 16]);
        let revision = zterm_core::Revision::new(7);
        let snapshot = TerminalSurfaceSnapshot {
            revision,
            surface: TerminalSurface {
                size: TerminalSize::new(2, 3),
                active_screen: ActiveScreen::Main,
                rows: vec![semantic_row(3, SENTINEL), semantic_row(3, "x")],
                cursor: TerminalCursor {
                    row: 1,
                    column: 2,
                    visible: true,
                    style: TerminalStyle::default(),
                },
                modes: TerminalModes::default(),
                scroll_metrics: Some(TerminalScrollMetrics {
                    epoch: zterm_core::Revision::new(1),
                    revision,
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 5,
                    viewport_rows: 2,
                }),
            },
        };
        let message =
            terminal_surface_snapshot_message(session_id, attachment_id, snapshot.clone());
        let debug = format!("{message:?}");
        assert!(!debug.contains(SENTINEL));
        let (decoded_session, decoded_attachment, decoded) =
            terminal_surface_snapshot_from_message(message.clone()).expect("valid snapshot");
        assert_eq!(decoded_session, session_id);
        assert_eq!(decoded_attachment, attachment_id);
        assert_eq!(decoded, snapshot);
        assert_message_round_trip(WireKind::TerminalSemanticSnapshot, message);

        let delta = TerminalSurfaceDelta {
            from_revision: revision,
            to_revision: zterm_core::Revision::new(8),
            size: snapshot.surface.size,
            active_screen: ActiveScreen::Main,
            row_patches: vec![TerminalSurfaceRowPatch {
                row: 1,
                replacement: semantic_row(3, "y"),
            }],
            cursor: snapshot.surface.cursor,
            modes: snapshot.surface.modes,
            scroll_metrics: Some(TerminalScrollMetrics {
                revision: zterm_core::Revision::new(8),
                ..snapshot.surface.scroll_metrics.expect("metrics")
            }),
        };
        let message = terminal_surface_delta_message(attachment_id, delta.clone());
        let (decoded_attachment, decoded) =
            terminal_surface_delta_from_message(message.clone()).expect("valid delta");
        assert_eq!(decoded_attachment, attachment_id);
        assert_eq!(decoded, delta);
        assert_message_round_trip(WireKind::TerminalSemanticDelta, message);

        let mut malformed = terminal_surface_snapshot_message(session_id, attachment_id, snapshot);
        malformed.surface.as_mut().expect("surface").rows[0].cells[0].contents =
            "bad\u{1b}cell".to_owned();
        assert!(matches!(
            terminal_surface_snapshot_from_message(malformed),
            Err(ProtocolError::InvalidTerminalSurface(
                TerminalSurfaceError::InvalidCellText
            ))
        ));
    }

    #[test]
    fn clipboard_write_round_trips_as_decoded_redacted_content() {
        const SENTINEL: &str = "CLIPBOARD_3f7a";
        let attachment_id = AttachmentId::from_array([7; 16]);
        let write = TerminalClipboardWrite::new(SENTINEL.to_owned()).expect("valid write");
        let message = terminal_clipboard_write_message(attachment_id, write.clone());

        let debug = format!("{message:?}");
        assert!(!debug.contains(SENTINEL));
        assert!(debug.contains("[REDACTED]"));
        let (decoded_attachment, decoded) =
            terminal_clipboard_write_from_message(message.clone()).expect("valid message");
        assert_eq!(decoded_attachment, attachment_id);
        assert_eq!(decoded, write);
        assert_message_round_trip(WireKind::TerminalClipboardWrite, message);

        let invalid = v2::TerminalClipboardWrite {
            attachment_id: Some(attachment_id.into()),
            text: "bad\0clipboard".to_owned(),
        };
        assert!(matches!(
            terminal_clipboard_write_from_message(invalid),
            Err(ProtocolError::InvalidTerminalSemanticField(
                "clipboard_text"
            ))
        ));

        for invalid in [
            v2::TerminalClipboardWrite {
                attachment_id: None,
                text: "missing owner".to_owned(),
            },
            v2::TerminalClipboardWrite {
                attachment_id: Some(attachment_id.into()),
                text: String::new(),
            },
            v2::TerminalClipboardWrite {
                attachment_id: Some(attachment_id.into()),
                text: "x".repeat(MAX_TERMINAL_CLIPBOARD_BYTES + 1),
            },
        ] {
            assert!(terminal_clipboard_write_from_message(invalid).is_err());
        }

        let maximum = terminal_clipboard_write_message(
            attachment_id,
            TerminalClipboardWrite::new("x".repeat(MAX_TERMINAL_CLIPBOARD_BYTES))
                .expect("exact maximum clipboard value"),
        );
        let encoded = encode_message(WireKind::TerminalClipboardWrite, 0, 0, &maximum)
            .expect("exact maximum clipboard frame fits the control cap");
        let decoded = FrameDecoder::new()
            .feed(&encoded)
            .expect("exact maximum clipboard frame decodes")
            .pop()
            .expect("one exact maximum clipboard frame");
        assert_eq!(decoded.kind, WireKind::TerminalClipboardWrite);
        assert!(decoded.payload.len() <= MAX_CONTROL_PAYLOAD_BYTES);
        let maximum: v2::TerminalClipboardWrite = decoded
            .decode_message(WireKind::TerminalClipboardWrite)
            .expect("exact maximum clipboard message decodes");
        assert_eq!(maximum.text.len(), MAX_TERMINAL_CLIPBOARD_BYTES);
    }

    #[test]
    fn terminal_keyboard_flags_round_trip_and_reject_unknown_bits() {
        let flags = TerminalKeyboardFlags::DISAMBIGUATE_ESCAPE_CODES
            .union(TerminalKeyboardFlags::REPORT_EVENT_TYPES)
            .union(TerminalKeyboardFlags::REPORT_ASSOCIATED_TEXT);
        let modes = TerminalModes {
            keyboard_flags: flags,
            ..TerminalModes::default()
        };
        assert_eq!(
            TerminalModes::try_from(v2::TerminalModes::from(modes)).expect("valid flags"),
            modes
        );

        let malformed = v2::TerminalModes {
            keyboard_flags: 32,
            ..v2::TerminalModes::default()
        };
        assert!(matches!(
            TerminalModes::try_from(malformed),
            Err(ProtocolError::InvalidTerminalSemanticField(
                "keyboard_flags"
            ))
        ));
    }

    #[test]
    fn maximum_semantic_frames_fit_the_existing_eight_mib_bound() {
        let attachment_id = AttachmentId::from_array([9; 16]);
        let session_id = SessionId::from_array([8; 16]);
        let revision = zterm_core::Revision::new(11);
        let size = TerminalSize::new(80, 240);
        let row = semantic_row(size.columns, "abcdefghijklmnopqrstuv");
        let surface = TerminalSurface {
            size,
            active_screen: ActiveScreen::Main,
            rows: vec![row.clone(); usize::from(size.rows)],
            cursor: TerminalCursor {
                row: 79,
                column: 239,
                visible: true,
                style: TerminalStyle::default(),
            },
            modes: TerminalModes::default(),
            scroll_metrics: Some(TerminalScrollMetrics {
                epoch: zterm_core::Revision::new(1),
                revision,
                offset_from_bottom: 0,
                max_offset_from_bottom: 240,
                viewport_rows: 80,
            }),
        };
        let snapshot = terminal_surface_snapshot_message(
            session_id,
            attachment_id,
            TerminalSurfaceSnapshot { revision, surface },
        );
        let encoded = encode_message(WireKind::TerminalSemanticSnapshot, 1, 0, &snapshot)
            .expect("maximum legal semantic snapshot fits");
        assert!(encoded.len() <= MAX_FRAME_BYTES + MAX_VARINT_BYTES);

        let query = TerminalHistoryWindowQuery {
            anchor: TerminalHistoryWindowAnchor {
                epoch: zterm_core::Revision::new(1),
                revision,
                max_offset_from_bottom: 240,
                viewport: size,
            },
            target_offset_from_bottom: 160,
            older_margin_rows: 80,
            newer_margin_rows: 80,
        };
        let frame = TerminalSurfaceHistoryWindowFrame {
            disposition: TerminalViewportDisposition::Exact,
            anchor: query.anchor,
            target_offset_from_bottom: 160,
            first_row_from_live_top: -240,
            rows: vec![row; zterm_core::terminal::MAX_HISTORY_WINDOW_ROWS],
        };
        frame.validate_for(query).expect("maximum window shape");
        let message = terminal_surface_history_window_frame_message(
            attachment_id,
            TerminalSurfaceHistoryWindowResult::Frame(frame),
        );
        let encoded = encode_message(WireKind::TerminalSemanticHistoryWindowFrame, 2, 0, &message)
            .expect("maximum legal semantic history window fits");
        assert!(encoded.len() <= MAX_FRAME_BYTES + MAX_VARINT_BYTES);
    }

    #[test]
    fn semantic_history_frames_keep_content_and_control_limits_and_reject_malformed_payloads() {
        assert!(WireKind::TerminalHistoryWindowRequest.is_control());
        assert!(!WireKind::TerminalSemanticSnapshot.is_control());
        assert!(!WireKind::TerminalSemanticDelta.is_control());
        assert!(!WireKind::TerminalSemanticHistoryWindowFrame.is_control());
        assert!(WireKind::TerminalConnectionStatusEvent.is_control());
        assert!(WireKind::TerminalClipboardWrite.is_control());

        let content = vec![0_u8; MAX_CONTROL_PAYLOAD_BYTES + 1];
        let encoded = encode_payload(
            WireKind::TerminalSemanticHistoryWindowFrame,
            7,
            0,
            content.clone(),
        )
        .expect("semantic history content may use the ordinary frame ceiling");
        assert_eq!(
            FrameDecoder::new()
                .feed(&encoded)
                .expect("bounded semantic history content frame")
                .len(),
            1
        );
        assert!(matches!(
            encode_payload(WireKind::TerminalHistoryWindowRequest, 7, 0, content),
            Err(ProtocolError::ControlPayloadTooLarge(_))
        ));

        let malformed = encode_payload(
            WireKind::TerminalSemanticHistoryWindowFrame,
            8,
            0,
            vec![0x80],
        )
        .expect("malformed concrete protobuf still has a valid outer frame");
        let frame = FrameDecoder::new()
            .feed(&malformed)
            .expect("outer frame decodes")
            .pop()
            .expect("one malformed concrete payload");
        assert!(matches!(
            frame.decode_message::<v2::TerminalSemanticHistoryWindowFrame>(
                WireKind::TerminalSemanticHistoryWindowFrame
            ),
            Err(ProtocolError::MalformedProtobuf(_))
        ));

        const SENTINEL: &str = "SECRET742f";
        let query = TerminalHistoryWindowQuery {
            anchor: TerminalHistoryWindowAnchor {
                epoch: zterm_core::Revision::new(1),
                revision: zterm_core::Revision::new(2),
                max_offset_from_bottom: 1,
                viewport: TerminalSize::new(1, 8),
            },
            target_offset_from_bottom: 1,
            older_margin_rows: 0,
            newer_margin_rows: 0,
        };
        let message = terminal_surface_history_window_frame_message(
            AttachmentId::from_array([0x47; 16]),
            TerminalSurfaceHistoryWindowResult::Frame(TerminalSurfaceHistoryWindowFrame {
                disposition: TerminalViewportDisposition::Exact,
                anchor: query.anchor,
                target_offset_from_bottom: 1,
                first_row_from_live_top: -1,
                rows: vec![semantic_row(8, SENTINEL)],
            }),
        );
        let debug = format!("{message:?}");
        assert!(!debug.contains(SENTINEL));
        let (_, decoded) =
            terminal_surface_history_window_from_message(message, query).expect("valid window");
        assert!(matches!(
            decoded,
            TerminalSurfaceHistoryWindowResult::Frame(_)
        ));

        for (current_epoch, current_revision) in [(3, 2), (0, 0)] {
            let malformed = v2::TerminalSemanticHistoryWindowFrame {
                attachment_id: Some(AttachmentId::from_array([0x48; 16]).into()),
                outcome: v2::TerminalHistoryWindowOutcome::Gap as i32,
                disposition: v2::TerminalViewportDisposition::Unspecified as i32,
                anchor: None,
                target_offset_from_bottom: 0,
                first_row_from_live_top: 0,
                rows: Vec::new(),
                current_epoch,
                current_revision,
            };
            assert!(matches!(
                terminal_surface_history_window_from_message(malformed, query),
                Err(ProtocolError::InvalidTerminalSurface(
                    TerminalSurfaceError::InvalidHistoryWindow
                ))
            ));
        }
    }
}
