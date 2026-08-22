//! Typed M3 local-daemon service dispatch.

#[cfg(unix)]
use std::time::{SystemTime, UNIX_EPOCH};

use zterm_core::DeviceId;
#[cfg(unix)]
use zterm_core::{Capabilities, DomainErrorKind};
#[cfg(unix)]
use zterm_proto::{DecodedFrame, WireKind, encode_message, v1};

use crate::bootstrap::BootstrapResult;
#[cfg(unix)]
use crate::config::{ValidatedConfig, validate_setup_profile};
#[cfg(unix)]
use crate::error::DaemonError;

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
    /// Live terminal sessions; structurally zero until M4.
    pub active_session_count: u32,
    /// Live terminal session names; empty until M4.
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
    /// Live terminal sessions; structurally zero until M4.
    pub active_session_count: u32,
    /// Live terminal session names; empty until M4.
    pub active_session_names: Vec<String>,
    /// Whether the accepted stop request is shutting the daemon down.
    pub stopping: bool,
    /// Whether a future manual update would interrupt work.
    pub interruption_required: bool,
}

/// Immutable M3 service state for one daemon process.
#[derive(Clone, Debug)]
pub struct DaemonService {
    #[cfg(unix)]
    setup: BootstrapResult,
    #[cfg(unix)]
    started_at_unix: u64,
}

impl DaemonService {
    /// Creates service state from already validated persistent setup.
    #[cfg(unix)]
    #[must_use]
    pub fn new(setup: BootstrapResult) -> Self {
        Self::with_started_at(setup, now_unix())
    }

    /// Creates the non-Unix placeholder for the M3 unsupported boundary.
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
        Self {
            setup,
            started_at_unix,
        }
    }

    /// Creates the non-Unix placeholder for isolated cross-platform callers.
    #[cfg(not(unix))]
    #[doc(hidden)]
    #[must_use]
    pub fn with_started_at(_setup: BootstrapResult, _started_at_unix: u64) -> Self {
        Self {}
    }

    /// Dispatches one validated local request frame.
    #[cfg(unix)]
    pub(crate) async fn dispatch(&self, frame: DecodedFrame) -> ServiceReply {
        let request_id = frame.request_id;
        let result = self.dispatch_inner(frame);
        match result {
            Ok(reply) => reply,
            Err(error) => ServiceReply::error(request_id, &error),
        }
    }

    #[cfg(unix)]
    fn dispatch_inner(&self, frame: DecodedFrame) -> Result<ServiceReply, DaemonError> {
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
                        active_session_count: 0,
                        active_session_names: Vec::new(),
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
                ServiceReply::message(
                    WireKind::LocalStopResponse,
                    request_id,
                    &v1::LocalStopResponse {
                        active_session_count: 0,
                        active_session_names: Vec::new(),
                        stopping: true,
                    },
                    true,
                )
            }
            WireKind::LocalUpdatePreflightRequest => {
                let _: v1::LocalUpdatePreflightRequest = decode_request(&frame)?;
                ServiceReply::message(
                    WireKind::LocalUpdatePreflightResponse,
                    request_id,
                    &v1::LocalUpdatePreflightResponse {
                        active_session_count: 0,
                        active_session_names: Vec::new(),
                        interruption_required: false,
                    },
                    false,
                )
            }
            _ => Err(DaemonError::new(
                DomainErrorKind::ServiceNotImplemented,
                format!("wire service {:?} is not implemented in M3", frame.kind),
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
        | ProtocolError::InvalidIdentifier(_) => DomainErrorKind::MalformedFrame,
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
        capabilities: Capabilities::LOCAL_LIFECYCLE,
    }
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
