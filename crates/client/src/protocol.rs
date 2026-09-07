//! Bounded decoding shared by local adapters and authenticated remote streams.
use crate::{
    error::ClientError,
    model::{ResolvedSessionTarget, SessionSummary, session_summary_from_wire},
};
use zterm_core::DomainErrorKind;
use zterm_proto::{DecodedFrame, WireKind, v2};

/// Maps protocol validation failure to its stable domain category.
pub fn protocol_error(error: zterm_proto::ProtocolError) -> ClientError {
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
        | ProtocolError::InvalidTerminalSize { .. }
        | ProtocolError::InvalidTerminalSurface(_)
        | ProtocolError::InvalidTerminalSemanticField(_) => DomainErrorKind::MalformedFrame,
    };
    ClientError::new(kind, error.to_string())
}

/// Constructs a bounded malformed-frame diagnostic.
pub fn malformed(detail: impl Into<String>) -> ClientError {
    ClientError::new(DomainErrorKind::MalformedFrame, detail)
}

/// Constructs a bounded resource-limit diagnostic.
pub fn resource_error(detail: impl Into<String>) -> ClientError {
    ClientError::new(DomainErrorKind::ResourceExhausted, detail)
}

/// Decodes a peer service failure and rejects unknown category codes.
pub fn service_error(frame: &DecodedFrame) -> Result<ClientError, ClientError> {
    let service_error: v2::ServiceError = frame
        .decode_message(WireKind::ServiceErrorResponse)
        .map_err(protocol_error)?;
    let kind = DomainErrorKind::from_code(&service_error.code).ok_or_else(|| {
        malformed(format!(
            "local daemon returned unknown error code {:?}",
            service_error.code
        ))
    })?;
    Ok(ClientError::new(kind, service_error.message))
}

/// Decodes the payload of an already correlated frame.
pub fn decode_response<Message>(frame: &DecodedFrame) -> Result<Message, ClientError>
where
    Message: prost::Message + Default,
{
    frame.decode_message(frame.kind).map_err(protocol_error)
}

/// Encodes an exact target without re-resolving mutable aliases.
pub fn resolved_target_wire(target: ResolvedSessionTarget) -> v2::TargetSelector {
    let target = match target.device_id() {
        Some(device_id) => v2::target_selector::Target::Device(device_id.into()),
        None => v2::target_selector::Target::Local(true),
    };
    v2::TargetSelector {
        target: Some(target),
    }
}

/// Validates the semantic result of an already correlated mutation.
pub fn mutate_response(frame: DecodedFrame) -> Result<SessionSummary, ClientError> {
    let response: v2::SessionMutateResponse = decode_response(&frame)?;
    session_summary_from_wire(
        response
            .session
            .ok_or_else(|| malformed("session mutation response omitted session"))?,
    )
}

/// Decodes the exact target returned by the local resolver.
pub fn resolved_target_from_wire(
    target: Option<v2::TargetSelector>,
) -> Result<ResolvedSessionTarget, ClientError> {
    match target.and_then(|target| target.target) {
        Some(v2::target_selector::Target::Local(true)) => Ok(ResolvedSessionTarget::local()),
        Some(v2::target_selector::Target::Device(device_id)) => {
            let device_id = device_id.try_into().map_err(protocol_error)?;
            Ok(ResolvedSessionTarget::device(device_id))
        }
        _ => Err(malformed(
            "target resolution response omitted a valid frozen target",
        )),
    }
}

use std::time::Duration;
/// Default absolute control-operation budget; idle streams have no timeout.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(5);
const ATTACHMENT_COMMAND_STREAM_CLOSED: &str = "local terminal attachment command stream closed";

/// Creates a bounded control-operation deadline.
pub fn control_deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + DEFAULT_DEADLINE
}

/// Returns the stable timeout diagnostic.
pub fn control_timeout() -> ClientError {
    ClientError::new(
        DomainErrorKind::DeadlineExceeded,
        "terminal control operation exceeded its deadline",
    )
}

/// Classifies an attachment which closed without a terminal event.
pub fn attachment_cancelled() -> ClientError {
    ClientError::new(
        DomainErrorKind::Cancelled,
        "local terminal attachment closed",
    )
}

/// Recognizes a half-closed command stream which may still report its outcome.
pub fn is_attachment_command_stream_closed(error: &ClientError) -> bool {
    error.kind() == DomainErrorKind::Cancelled && error.detail() == ATTACHMENT_COMMAND_STREAM_CLOSED
}

/// Recognizes an attachment EOF without a typed outcome.
pub fn is_attachment_stream_closed_without_event(error: &ClientError) -> bool {
    error.kind() == DomainErrorKind::Cancelled
        && error.detail() == "local terminal attachment closed"
}

/// Classifies a closed write half while preserving a bounded read-drain opportunity.
pub fn attachment_command_stream_closed() -> ClientError {
    ClientError::new(DomainErrorKind::Cancelled, ATTACHMENT_COMMAND_STREAM_CLOSED)
}
