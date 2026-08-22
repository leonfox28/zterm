//! Typed local-daemon lifecycle and session-service dispatch.

#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use zterm_core::DeviceId;
#[cfg(unix)]
use zterm_core::{
    AttachmentId, Capabilities, DomainErrorKind, OperationId, SessionId, SessionName,
};
#[cfg(unix)]
use zterm_proto::{DecodedFrame, WireKind, encode_message, v1};

use crate::bootstrap::BootstrapResult;
#[cfg(unix)]
use crate::config::{ValidatedConfig, validate_setup_profile};
#[cfg(unix)]
use crate::error::DaemonError;
#[cfg(unix)]
use crate::session::{SessionService, SessionSummary};

/// Protocol version projected by readiness and status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolStatus {
    /// Product wire major.
    pub wire_major: u32,
    /// Persistent-state schema supported by this binary.
    pub state_schema: u32,
    /// Negotiable capability bits, retaining future unknown values.
    pub capabilities: u64,
}

/// Successful local readiness projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonReadiness {
    /// Supported protocol values.
    pub protocol: ProtocolStatus,
    /// Running package version.
    pub version: String,
    /// Daemon process start timestamp.
    pub started_at_unix: u64,
}

/// Current daemon status shared by CLI human and JSON renderers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonStatus {
    /// Supported protocol values.
    pub protocol: ProtocolStatus,
    /// Running package version.
    pub version: String,
    /// Current implementation phase.
    pub phase: String,
    /// Stable public device identity bytes.
    pub device_id: DeviceId,
    /// Iroh's canonical public endpoint encoding.
    pub endpoint_id: String,
    /// User-facing device name.
    pub device_name: String,
    /// Selected infrastructure profile name.
    pub infrastructure_profile: String,
    /// Daemon process start timestamp.
    pub started_at_unix: u64,
    /// Live terminal sessions in the current daemon.
    pub active_session_count: u32,
    /// Live terminal session names in stable display order.
    pub active_session_names: Vec<String>,
}

/// Result of validating setup against the running daemon.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedSetupStatus {
    /// Stable public device identity bytes.
    pub device_id: DeviceId,
    /// Iroh's canonical public endpoint encoding.
    pub endpoint_id: String,
}

/// Active-session impact returned by stop and update preflight.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionImpact {
    /// Live terminal sessions affected by the operation.
    pub active_session_count: u32,
    /// Live terminal session names affected by the operation.
    pub active_session_names: Vec<String>,
    /// Whether the accepted stop request is shutting the daemon down.
    pub stopping: bool,
    /// Whether a future manual update would interrupt work.
    pub interruption_required: bool,
}

/// Shared lifecycle and live-session service state for one daemon process.
#[derive(Clone)]
pub struct DaemonService {
    #[cfg(unix)]
    setup: BootstrapResult,
    #[cfg(unix)]
    started_at_unix: u64,
    #[cfg(unix)]
    sessions: SessionService,
}

impl DaemonService {
    /// Creates service state from already validated persistent setup.
    #[cfg(unix)]
    #[must_use]
    pub fn new(setup: BootstrapResult) -> Self {
        Self::with_started_at(setup, now_unix())
    }

    /// Creates the non-Unix placeholder for the current unsupported boundary.
    #[cfg(not(unix))]
    #[must_use]
    pub fn new(_setup: BootstrapResult) -> Self {
        Self {}
    }

    /// Creates service state with an explicit timestamp for isolated tests.
    #[cfg(unix)]
    #[doc(hidden)]
    #[must_use]
    pub fn with_started_at(setup: BootstrapResult, started_at_unix: u64) -> Self {
        let sessions = SessionService::new(setup.device_id);
        Self {
            setup,
            started_at_unix,
            sessions,
        }
    }

    /// Creates isolated service state around a task-private session service.
    #[cfg(unix)]
    #[doc(hidden)]
    #[must_use]
    pub fn with_sessions(
        setup: BootstrapResult,
        started_at_unix: u64,
        sessions: SessionService,
    ) -> Self {
        Self {
            setup,
            started_at_unix,
            sessions,
        }
    }

    #[cfg(unix)]
    pub(crate) const fn sessions(&self) -> &SessionService {
        &self.sessions
    }

    /// Creates the non-Unix placeholder for isolated cross-platform callers.
    #[cfg(not(unix))]
    #[doc(hidden)]
    #[must_use]
    pub fn with_started_at(_setup: BootstrapResult, _started_at_unix: u64) -> Self {
        Self {}
    }

    /// Dispatches without blocking the daemon runtime thread.
    #[cfg(unix)]
    pub(crate) async fn dispatch_until(
        &self,
        frame: DecodedFrame,
        deadline: Instant,
    ) -> ServiceReply {
        let request_id = frame.request_id;
        let service = self.clone();
        let result = tokio::task::spawn_blocking(move || service.dispatch_inner(frame, deadline))
            .await
            .map_err(|error| {
                DaemonError::new(
                    DomainErrorKind::Cancelled,
                    format!("local service worker ended unexpectedly: {error}"),
                )
            })
            .and_then(|result| result);
        match result {
            Ok(reply) => reply,
            Err(error) => ServiceReply::error(request_id, &error),
        }
    }

    #[cfg(unix)]
    fn dispatch_inner(
        &self,
        frame: DecodedFrame,
        deadline: Instant,
    ) -> Result<ServiceReply, DaemonError> {
        let request_id = frame.request_id;
        match frame.kind {
            WireKind::LocalReadinessRequest => {
                let _: v1::LocalReadinessRequest = decode_request(&frame)?;
                ServiceReply::message(
                    WireKind::LocalReadinessResponse,
                    request_id,
                    &v1::LocalReadinessResponse {
                        protocol: Some(protocol_proto()),
                        version: env!("CARGO_PKG_VERSION").to_owned(),
                        started_at_unix: self.started_at_unix,
                    },
                    false,
                )
            }
            WireKind::LocalStatusRequest => {
                let _: v1::LocalStatusRequest = decode_request(&frame)?;
                let sessions = self.sessions.list()?;
                let active_session_names = session_names(&sessions);
                ServiceReply::message(
                    WireKind::LocalStatusResponse,
                    request_id,
                    &v1::LocalStatusResponse {
                        protocol: Some(protocol_proto()),
                        version: env!("CARGO_PKG_VERSION").to_owned(),
                        phase: zterm_core::PHASE_NAME.to_owned(),
                        device_id: Some(self.setup.device_id.into()),
                        endpoint_id: self.setup.endpoint_id.clone(),
                        device_name: self.setup.config.device_name.clone(),
                        infrastructure_profile: self
                            .setup
                            .config
                            .infrastructure
                            .profile_name()
                            .to_owned(),
                        started_at_unix: self.started_at_unix,
                        active_session_count: u32::try_from(sessions.len()).unwrap_or(u32::MAX),
                        active_session_names,
                    },
                    false,
                )
            }
            WireKind::LocalValidateSetupRequest => {
                let request: v1::LocalValidateSetupRequest = decode_request(&frame)?;
                let requested = config_from_wire(&request)?;
                if requested != self.setup.config {
                    return Err(DaemonError::new(
                        DomainErrorKind::AlreadyConfiguredConflict,
                        "requested setup differs from the running daemon configuration",
                    ));
                }
                ServiceReply::message(
                    WireKind::LocalValidateSetupResponse,
                    request_id,
                    &v1::LocalValidateSetupResponse {
                        device_id: Some(self.setup.device_id.into()),
                        endpoint_id: self.setup.endpoint_id.clone(),
                    },
                    false,
                )
            }
            WireKind::LocalStopRequest => {
                let _: v1::LocalStopRequest = decode_request(&frame)?;
                let sessions = self.sessions.shutdown_until(deadline)?;
                ServiceReply::message(
                    WireKind::LocalStopResponse,
                    request_id,
                    &v1::LocalStopResponse {
                        active_session_count: u32::try_from(sessions.len()).unwrap_or(u32::MAX),
                        active_session_names: session_names(&sessions),
                        stopping: true,
                    },
                    true,
                )
            }
            WireKind::LocalUpdatePreflightRequest => {
                let _: v1::LocalUpdatePreflightRequest = decode_request(&frame)?;
                let sessions = self.sessions.list()?;
                ServiceReply::message(
                    WireKind::LocalUpdatePreflightResponse,
                    request_id,
                    &v1::LocalUpdatePreflightResponse {
                        active_session_count: u32::try_from(sessions.len()).unwrap_or(u32::MAX),
                        active_session_names: session_names(&sessions),
                        interruption_required: !sessions.is_empty(),
                    },
                    false,
                )
            }
            WireKind::SessionListRequest => {
                let request: v1::SessionListRequest = decode_request(&frame)?;
                require_local_target(request.target)?;
                let sessions = self
                    .sessions
                    .list()?
                    .into_iter()
                    .map(session_summary_proto)
                    .collect();
                ServiceReply::message(
                    WireKind::SessionListResponse,
                    request_id,
                    &v1::SessionListResponse { sessions },
                    false,
                )
            }
            WireKind::SessionOperationLeaseRequest => {
                let request: v1::SessionOperationLeaseRequest = decode_request(&frame)?;
                require_local_target(request.target)?;
                let lease = self
                    .sessions
                    .issue_operation_lease(local_principal(self.setup.device_id, request_id))?;
                ServiceReply::message(
                    WireKind::SessionOperationLeaseResponse,
                    request_id,
                    &v1::SessionOperationLeaseResponse {
                        lease: Some(lease.into()),
                    },
                    false,
                )
            }
            WireKind::SessionCreateRequest => {
                let request: v1::SessionCreateRequest = decode_request(&frame)?;
                require_local_target(request.target)?;
                let operation_id = required_operation_id(request.operation_id)?;
                let name = session_name(&request.name)?;
                let working_directory = (!request.working_directory.is_empty())
                    .then(|| PathBuf::from(request.working_directory));
                let viewport = request
                    .viewport
                    .map(TryInto::try_into)
                    .transpose()
                    .map_err(protocol_error)?;
                let summary = self.sessions.create_until(
                    local_principal(self.setup.device_id, request_id),
                    operation_id,
                    name,
                    working_directory,
                    viewport,
                    deadline,
                )?;
                mutate_reply(request_id, summary)
            }
            WireKind::SessionRenameRequest => {
                let request: v1::SessionRenameRequest = decode_request(&frame)?;
                require_local_target(request.target)?;
                let summary = self.sessions.rename_until(
                    local_principal(self.setup.device_id, request_id),
                    required_operation_id(request.operation_id)?,
                    required_session_id(request.session_id)?,
                    session_name(&request.name)?,
                    deadline,
                )?;
                mutate_reply(request_id, summary)
            }
            WireKind::SessionCloseRequest => {
                let request: v1::SessionCloseRequest = decode_request(&frame)?;
                require_local_target(request.target)?;
                let summary = self.sessions.close_until(
                    local_principal(self.setup.device_id, request_id),
                    required_operation_id(request.operation_id)?,
                    required_session_id(request.session_id)?,
                    deadline,
                )?;
                mutate_reply(request_id, summary)
            }
            WireKind::SessionTakeoverRequest => {
                let request: v1::SessionTakeoverRequest = decode_request(&frame)?;
                require_local_target(request.target)?;
                let attachment_id = request
                    .attachment_id
                    .ok_or_else(|| malformed("takeover omitted attachment_id"))?
                    .try_into()
                    .map_err(protocol_error)?;
                let summary = self.sessions.takeover_by_id_until(
                    local_principal(self.setup.device_id, request_id),
                    required_operation_id(request.operation_id)?,
                    required_session_id(request.session_id)?,
                    attachment_id,
                    deadline,
                )?;
                mutate_reply(request_id, summary)
            }
            _ => Err(DaemonError::new(
                DomainErrorKind::ServiceNotImplemented,
                format!("wire service {:?} is not implemented in M4", frame.kind),
            )),
        }
    }
}

#[cfg(unix)]
pub(crate) struct ServiceReply {
    pub(crate) bytes: Vec<u8>,
    pub(crate) stop_after_flush: bool,
}

#[cfg(unix)]
impl ServiceReply {
    fn message<Message>(
        kind: WireKind,
        request_id: u64,
        message: &Message,
        stop_after_flush: bool,
    ) -> Result<Self, DaemonError>
    where
        Message: prost::Message,
    {
        let bytes = encode_message(kind, request_id, 0, message).map_err(protocol_error)?;
        Ok(Self {
            bytes,
            stop_after_flush,
        })
    }

    pub(crate) fn error(request_id: u64, error: &DaemonError) -> Self {
        let message = v1::ServiceError {
            code: error.kind().code().to_owned(),
            message: error.detail().to_owned(),
        };
        let bytes = encode_message(WireKind::ServiceErrorResponse, request_id, 0, &message)
            .expect("bounded daemon errors always fit the service-error frame");
        Self {
            bytes,
            stop_after_flush: false,
        }
    }
}

#[cfg(unix)]
pub(crate) fn protocol_error(error: zterm_proto::ProtocolError) -> DaemonError {
    use zterm_proto::ProtocolError;
    let kind = match error {
        ProtocolError::WireMajorMismatch { .. } => DomainErrorKind::WireMajorMismatch,
        ProtocolError::UnknownKind(_) => DomainErrorKind::UnknownKind,
        ProtocolError::FrameTooLarge(_) => DomainErrorKind::FrameTooLarge,
        ProtocolError::ControlPayloadTooLarge(_) => DomainErrorKind::ControlPayloadTooLarge,
        ProtocolError::MalformedVarint
        | ProtocolError::TruncatedFrame
        | ProtocolError::MalformedProtobuf(_)
        | ProtocolError::UnexpectedKind { .. }
        | ProtocolError::InvalidIdentifier(_)
        | ProtocolError::InvalidTerminalSize { .. } => DomainErrorKind::MalformedFrame,
    };
    DaemonError::new(kind, error.to_string())
}

#[cfg(unix)]
fn decode_request<Message>(frame: &DecodedFrame) -> Result<Message, DaemonError>
where
    Message: prost::Message + Default,
{
    frame.decode_message(frame.kind).map_err(protocol_error)
}

#[cfg(unix)]
fn protocol_proto() -> v1::ProtocolVersion {
    v1::ProtocolVersion {
        wire_major: zterm_proto::WIRE_MAJOR,
        state_schema: zterm_proto::STATE_SCHEMA_VERSION,
        capabilities: Capabilities::LOCAL_LIFECYCLE
            | Capabilities::SESSION_SERVICE
            | Capabilities::TERMINAL_SERVICE,
    }
}

#[cfg(unix)]
fn mutate_reply(request_id: u64, summary: SessionSummary) -> Result<ServiceReply, DaemonError> {
    ServiceReply::message(
        WireKind::SessionMutateResponse,
        request_id,
        &v1::SessionMutateResponse {
            session: Some(session_summary_proto(summary)),
        },
        false,
    )
}

#[cfg(unix)]
fn session_summary_proto(summary: SessionSummary) -> v1::SessionSummary {
    v1::SessionSummary {
        session_id: Some(summary.session_id.into()),
        name: summary.name.to_string(),
        revision: summary.revision.get(),
        has_controller: summary.has_controller,
        working_directory: summary.working_directory.to_string_lossy().into_owned(),
        viewport: Some(summary.viewport.into()),
    }
}

#[cfg(unix)]
fn session_names(sessions: &[SessionSummary]) -> Vec<String> {
    sessions
        .iter()
        .map(|session| session.name.to_string())
        .collect()
}

#[cfg(unix)]
fn require_local_target(target: Option<v1::TargetSelector>) -> Result<(), DaemonError> {
    match target.and_then(|target| target.target) {
        Some(v1::target_selector::Target::Local(true)) => Ok(()),
        _ => Err(malformed(
            "local session request requires target.local=true",
        )),
    }
}

#[cfg(unix)]
fn required_operation_id(
    operation_id: Option<v1::OperationId>,
) -> Result<OperationId, DaemonError> {
    operation_id
        .ok_or_else(|| malformed("session mutation omitted operation_id"))?
        .try_into()
        .map_err(protocol_error)
}

#[cfg(unix)]
fn required_session_id(session_id: Option<v1::SessionId>) -> Result<SessionId, DaemonError> {
    session_id
        .ok_or_else(|| malformed("session mutation omitted session_id"))?
        .try_into()
        .map_err(protocol_error)
}

#[cfg(unix)]
fn session_name(value: &str) -> Result<SessionName, DaemonError> {
    SessionName::new(value.to_owned())
        .map_err(|error| DaemonError::new(DomainErrorKind::InvalidSessionName, error.to_string()))
}

#[cfg(unix)]
fn local_principal(device_id: DeviceId, request_id: u64) -> zterm_core::AttachmentPrincipal {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&request_id.to_le_bytes());
    zterm_core::AttachmentPrincipal::LocalSameUid {
        own_device_id: device_id,
        local_view_id: AttachmentId::from_array(bytes),
    }
}

#[cfg(unix)]
fn malformed(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

#[cfg(unix)]
fn config_from_wire(
    request: &v1::LocalValidateSetupRequest,
) -> Result<ValidatedConfig, DaemonError> {
    let relay_url = (!request.relay_url.is_empty()).then_some(request.relay_url.as_str());
    validate_setup_profile(
        &request.device_name,
        &request.infrastructure_profile,
        relay_url,
    )
}

#[cfg(unix)]
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
