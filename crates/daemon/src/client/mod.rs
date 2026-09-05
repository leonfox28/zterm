//! Frontend-owned Session protocol, transport, and typed terminal view.
//!
//! The daemon server owns host Sessions and opaque remote tunnels. This boundary
//! owns frontend attachment epochs, correlation, recovery, and control budgets.

pub mod ipc;
#[cfg(unix)]
mod session;
#[cfg(unix)]
mod transport;
pub mod view;

#[cfg(unix)]
pub use ipc::LocalPairingClient;
pub use ipc::{LocalClient, LocalDeviceClient};
#[cfg(unix)]
pub(crate) use session::RemoteDaemonRestarter;
#[cfg(unix)]
pub use session::{LocalAttachmentEvent, LocalTakeoverRetryToken, SessionClient};

#[cfg(unix)]
use crate::{device_directory::ResolvedSessionTarget, error::DaemonError, service::protocol_error};
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use zterm_core::DomainErrorKind;
#[cfg(unix)]
use zterm_proto::{DecodedFrame, WireKind, v2};

#[cfg(unix)]
const DEFAULT_DEADLINE: Duration = Duration::from_secs(5);
#[cfg(unix)]
const ATTACHMENT_COMMAND_STREAM_CLOSED: &str = "local terminal attachment command stream closed";

#[cfg(unix)]
fn control_deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + DEFAULT_DEADLINE
}

#[cfg(unix)]
fn control_timeout() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DeadlineExceeded,
        "terminal control operation exceeded its deadline",
    )
}

#[cfg(unix)]
pub(super) fn connect_error(error: std::io::Error) -> DaemonError {
    let kind = match error.kind() {
        std::io::ErrorKind::PermissionDenied => DomainErrorKind::PermissionMismatch,
        _ => DomainErrorKind::DaemonStopped,
    };
    DaemonError::new(kind, format!("local daemon is unavailable: {error}"))
}

#[cfg(unix)]
pub(super) fn daemon_io(operation: &str, error: std::io::Error) -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DaemonStopped,
        format!("{operation}: {error}"),
    )
}

#[cfg(unix)]
pub(super) fn malformed(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

#[cfg(unix)]
pub(super) fn resource_error(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::ResourceExhausted, detail)
}

#[cfg(unix)]
pub(super) fn service_error(frame: &DecodedFrame) -> Result<DaemonError, DaemonError> {
    let service_error: v2::ServiceError = frame
        .decode_message(WireKind::ServiceErrorResponse)
        .map_err(protocol_error)?;
    let kind = DomainErrorKind::from_code(&service_error.code).ok_or_else(|| {
        malformed(format!(
            "local daemon returned unknown error code {:?}",
            service_error.code
        ))
    })?;
    Ok(DaemonError::new(kind, service_error.message))
}

#[cfg(unix)]
pub(super) fn decode_response<Message>(frame: &DecodedFrame) -> Result<Message, DaemonError>
where
    Message: prost::Message + Default,
{
    frame.decode_message(frame.kind).map_err(protocol_error)
}

#[cfg(unix)]
pub(super) fn resolved_target_wire(target: ResolvedSessionTarget) -> v2::TargetSelector {
    let target = match target.device_id() {
        Some(device_id) => v2::target_selector::Target::Device(device_id.into()),
        None => v2::target_selector::Target::Local(true),
    };
    v2::TargetSelector {
        target: Some(target),
    }
}

#[cfg(unix)]
pub(super) fn attachment_cancelled() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::Cancelled,
        "local terminal attachment closed",
    )
}

#[cfg(unix)]
pub(super) fn local_attachment_command_error(error: std::io::Error) -> DaemonError {
    if is_attachment_closure_error(error.kind()) {
        DaemonError::new(DomainErrorKind::Cancelled, ATTACHMENT_COMMAND_STREAM_CLOSED)
    } else {
        daemon_io("write local terminal message", error)
    }
}

#[cfg(unix)]
pub(super) fn local_attachment_io(operation: &str, error: std::io::Error) -> DaemonError {
    if is_attachment_closure_error(error.kind()) {
        attachment_cancelled()
    } else {
        daemon_io(operation, error)
    }
}

#[cfg(unix)]
pub(super) const fn is_attachment_closure_error(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::UnexpectedEof
    )
}

#[cfg(unix)]
pub(super) fn is_attachment_command_stream_closed(error: &DaemonError) -> bool {
    error.kind() == DomainErrorKind::Cancelled && error.detail() == ATTACHMENT_COMMAND_STREAM_CLOSED
}

#[cfg(unix)]
pub(super) fn is_attachment_stream_closed_without_event(error: &DaemonError) -> bool {
    error.kind() == DomainErrorKind::Cancelled
        && error.detail() == "local terminal attachment closed"
}
