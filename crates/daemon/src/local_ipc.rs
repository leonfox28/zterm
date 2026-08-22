//! Bounded same-UID unary IPC over Unix-domain sockets.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use zterm_core::{DomainErrorKind, ResourceLimits};
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v1};

use crate::config::ValidatedConfig;
use crate::error::DaemonError;
use crate::service::{
    DaemonReadiness, DaemonService, DaemonStatus, ProtocolStatus, ServiceReply, SessionImpact,
    ValidatedSetupStatus, protocol_error,
};

const DEFAULT_DEADLINE: Duration = Duration::from_secs(5);
const DRAIN_GRACE: Duration = Duration::from_secs(30);

/// Fixed production limits with a reduced test constructor for deadline evidence.
#[derive(Clone, Copy, Debug)]
pub struct LocalIpcLimits {
    initial_read_timeout: Duration,
    default_request_deadline: Duration,
    maximum_request_deadline: Duration,
    maximum_connections: usize,
}

impl Default for LocalIpcLimits {
    fn default() -> Self {
        let resources = ResourceLimits::default();
        Self {
            initial_read_timeout: DEFAULT_DEADLINE,
            default_request_deadline: DEFAULT_DEADLINE,
            maximum_request_deadline: Duration::from_secs(u64::from(
                resources.max_local_deadline_seconds,
            )),
            maximum_connections: resources.max_local_connections,
        }
    }
}

impl LocalIpcLimits {
    /// Creates bounded limits for deterministic isolated tests.
    #[doc(hidden)]
    #[must_use]
    pub const fn for_test(read_timeout: Duration) -> Self {
        Self {
            initial_read_timeout: read_timeout,
            default_request_deadline: read_timeout,
            maximum_request_deadline: read_timeout,
            maximum_connections: 32,
        }
    }

    fn request_deadline(self, requested_ms: u32) -> Duration {
        if requested_ms == 0 {
            self.default_request_deadline
        } else {
            Duration::from_millis(u64::from(requested_ms)).min(self.maximum_request_deadline)
        }
    }
}

/// Runs one local listener until a flushed stop response requests shutdown.
#[cfg(unix)]
pub async fn serve_local(
    listener: std::os::unix::net::UnixListener,
    expected_uid: u32,
    service: Arc<DaemonService>,
) -> Result<(), DaemonError> {
    serve_local_with_limits(listener, expected_uid, service, LocalIpcLimits::default()).await
}

/// Runs a listener with reduced limits for isolated protocol tests.
#[cfg(unix)]
#[doc(hidden)]
pub async fn serve_local_with_limits(
    listener: std::os::unix::net::UnixListener,
    expected_uid: u32,
    service: Arc<DaemonService>,
    limits: LocalIpcLimits,
) -> Result<(), DaemonError> {
    listener
        .set_nonblocking(true)
        .map_err(|error| daemon_io("configure local listener", error))?;
    let listener = tokio::net::UnixListener::from_std(listener)
        .map_err(|error| daemon_io("adopt local listener", error))?;
    let permits = Arc::new(Semaphore::new(limits.maximum_connections));
    let (stop_sender, mut stop_receiver) = mpsc::unbounded_channel();
    let mut handlers = JoinSet::new();

    loop {
        tokio::select! {
            biased;
            stop = stop_receiver.recv() => {
                if stop.is_some() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|error| daemon_io("accept local connection", error))?;
                let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let service = Arc::clone(&service);
                let stop_sender = stop_sender.clone();
                handlers.spawn(async move {
                    let _permit = permit;
                    handle_connection(stream, expected_uid, service, stop_sender, limits).await;
                });
            }
        }
    }

    drop(listener);
    drop(stop_sender);
    let drain = async { while handlers.join_next().await.is_some() {} };
    if tokio::time::timeout(DRAIN_GRACE, drain).await.is_err() {
        handlers.abort_all();
    }
    Ok(())
}

#[cfg(not(unix))]
/// Returns the M3 platform limitation on non-Unix targets.
pub async fn serve_local(
    _listener: (),
    _expected_uid: u32,
    _service: Arc<DaemonService>,
) -> Result<(), DaemonError> {
    Err(DaemonError::new(
        DomainErrorKind::UnsupportedPlatform,
        "local daemon IPC is Unix-only in M3",
    ))
}

#[cfg(unix)]
async fn handle_connection(
    mut stream: tokio::net::UnixStream,
    expected_uid: u32,
    service: Arc<DaemonService>,
    stop_sender: mpsc::UnboundedSender<()>,
    limits: LocalIpcLimits,
) {
    if zterm_platform::local_unix::authorize_stream_peer(&stream, expected_uid).is_err() {
        return;
    }

    let started = Instant::now();
    let frame = match tokio::time::timeout(limits.initial_read_timeout, read_one(&mut stream)).await
    {
        Ok(Ok(frame)) => frame,
        Ok(Err(error)) => {
            let _ = write_error(&mut stream, 0, &error).await;
            return;
        }
        Err(_) => {
            let error = DaemonError::new(
                DomainErrorKind::DeadlineExceeded,
                "local request frame read exceeded its deadline",
            );
            let _ = write_error(&mut stream, 0, &error).await;
            return;
        }
    };
    let request_id = frame.request_id;
    let deadline = limits.request_deadline(frame.deadline_ms);
    let remaining = deadline.saturating_sub(started.elapsed());
    let reply = match tokio::time::timeout(remaining, service.dispatch(frame)).await {
        Ok(reply) => reply,
        Err(_) => ServiceReply::error(
            request_id,
            &DaemonError::new(
                DomainErrorKind::DeadlineExceeded,
                "local request exceeded its deadline",
            ),
        ),
    };
    if stream.write_all(&reply.bytes).await.is_err() {
        return;
    }
    if stream.shutdown().await.is_err() {
        return;
    }
    if reply.stop_after_flush {
        let _ = stop_sender.send(());
    }
}

#[cfg(unix)]
async fn write_error(
    stream: &mut tokio::net::UnixStream,
    request_id: u64,
    error: &DaemonError,
) -> Result<(), std::io::Error> {
    stream
        .write_all(&ServiceReply::error(request_id, error).bytes)
        .await?;
    stream.shutdown().await
}

#[cfg(unix)]
async fn read_one(stream: &mut tokio::net::UnixStream) -> Result<DecodedFrame, DaemonError> {
    let mut decoder = FrameDecoder::new();
    let mut buffer = [0_u8; 16 * 1024];
    let mut completed = None;
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|error| daemon_io("read local request", error))?;
        if read == 0 {
            decoder.finish().map_err(protocol_error)?;
            return completed.ok_or_else(|| {
                DaemonError::new(
                    DomainErrorKind::Cancelled,
                    "local client closed before sending a request",
                )
            });
        }
        let frames = decoder.feed(&buffer[..read]).map_err(protocol_error)?;
        if frames.len() > 1 || (completed.is_some() && !frames.is_empty()) {
            return Err(DaemonError::new(
                DomainErrorKind::MalformedFrame,
                "one local connection may contain only one request",
            ));
        }
        if let Some(frame) = frames.into_iter().next() {
            completed = Some(frame);
        }
    }
}

/// Same-UID local daemon unary client. It never starts a daemon.
#[derive(Debug)]
pub struct LocalClient {
    socket: PathBuf,
    next_request_id: AtomicU64,
}

impl LocalClient {
    /// Creates a non-spawning client for one effective user's daemon socket.
    #[must_use]
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            next_request_id: AtomicU64::new(1),
        }
    }

    /// Returns the configured socket path without connecting.
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Probes daemon readiness.
    #[cfg(unix)]
    pub async fn readiness(&self) -> Result<DaemonReadiness, DaemonError> {
        let frame = self
            .request(
                WireKind::LocalReadinessRequest,
                WireKind::LocalReadinessResponse,
                &v1::LocalReadinessRequest {},
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v1::LocalReadinessResponse = decode_response(&frame)?;
        Ok(DaemonReadiness {
            protocol: protocol_status(response.protocol)?,
            version: response.version,
            started_at_unix: response.started_at_unix,
        })
    }

    /// Reads current daemon status.
    #[cfg(unix)]
    pub async fn status(&self) -> Result<DaemonStatus, DaemonError> {
        let frame = self
            .request(
                WireKind::LocalStatusRequest,
                WireKind::LocalStatusResponse,
                &v1::LocalStatusRequest {},
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v1::LocalStatusResponse = decode_response(&frame)?;
        let device_id = response
            .device_id
            .ok_or_else(|| malformed("status response omitted device_id"))?
            .try_into()
            .map_err(protocol_error)?;
        Ok(DaemonStatus {
            protocol: protocol_status(response.protocol)?,
            version: response.version,
            phase: response.phase,
            device_id,
            endpoint_id: response.endpoint_id,
            device_name: response.device_name,
            infrastructure_profile: response.infrastructure_profile,
            started_at_unix: response.started_at_unix,
            active_session_count: response.active_session_count,
            active_session_names: response.active_session_names,
        })
    }

    /// Validates requested setup against the running daemon without opening SQLite.
    #[cfg(unix)]
    pub async fn validate_setup(
        &self,
        requested: &ValidatedConfig,
    ) -> Result<ValidatedSetupStatus, DaemonError> {
        let frame = self
            .request(
                WireKind::LocalValidateSetupRequest,
                WireKind::LocalValidateSetupResponse,
                &v1::LocalValidateSetupRequest {
                    device_name: requested.device_name.clone(),
                    infrastructure_profile: requested.infrastructure.profile_name().to_owned(),
                    relay_url: requested
                        .infrastructure
                        .relay_url()
                        .map_or_else(String::new, ToString::to_string),
                },
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v1::LocalValidateSetupResponse = decode_response(&frame)?;
        let device_id = response
            .device_id
            .ok_or_else(|| malformed("validate-setup response omitted device_id"))?
            .try_into()
            .map_err(protocol_error)?;
        Ok(ValidatedSetupStatus {
            device_id,
            endpoint_id: response.endpoint_id,
        })
    }

    /// Requests graceful shutdown; the response is flushed before listener shutdown.
    #[cfg(unix)]
    pub async fn stop(&self, force: bool) -> Result<SessionImpact, DaemonError> {
        let frame = self
            .request(
                WireKind::LocalStopRequest,
                WireKind::LocalStopResponse,
                &v1::LocalStopRequest {
                    force,
                    operation_id: None,
                },
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v1::LocalStopResponse = decode_response(&frame)?;
        Ok(SessionImpact {
            active_session_count: response.active_session_count,
            active_session_names: response.active_session_names,
            stopping: response.stopping,
            interruption_required: false,
        })
    }

    /// Reads the schema-only manual-update impact without stopping the daemon.
    #[cfg(unix)]
    pub async fn update_preflight(&self) -> Result<SessionImpact, DaemonError> {
        let frame = self
            .request(
                WireKind::LocalUpdatePreflightRequest,
                WireKind::LocalUpdatePreflightResponse,
                &v1::LocalUpdatePreflightRequest {},
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v1::LocalUpdatePreflightResponse = decode_response(&frame)?;
        Ok(SessionImpact {
            active_session_count: response.active_session_count,
            active_session_names: response.active_session_names,
            stopping: false,
            interruption_required: response.interruption_required,
        })
    }

    #[cfg(unix)]
    async fn request<Message: prost::Message>(
        &self,
        request_kind: WireKind,
        response_kind: WireKind,
        message: &Message,
        deadline: Duration,
    ) -> Result<DecodedFrame, DaemonError> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let deadline_ms = u32::try_from(deadline.as_millis()).unwrap_or(u32::MAX);
        let bytes = encode_message(request_kind, request_id, deadline_ms, message)
            .map_err(protocol_error)?;
        let mut stream = tokio::net::UnixStream::connect(&self.socket)
            .await
            .map_err(connect_error)?;
        stream
            .write_all(&bytes)
            .await
            .map_err(|error| daemon_io("write local request", error))?;
        stream
            .shutdown()
            .await
            .map_err(|error| daemon_io("finish local request", error))?;
        let frame = tokio::time::timeout(deadline, read_one(&mut stream))
            .await
            .map_err(|_| {
                DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    "timed out waiting for local daemon response",
                )
            })??;
        if frame.request_id != request_id {
            return Err(malformed("local response request_id mismatch"));
        }
        if frame.kind == WireKind::ServiceErrorResponse {
            let service_error: v1::ServiceError = frame
                .decode_message(WireKind::ServiceErrorResponse)
                .map_err(protocol_error)?;
            let kind = DomainErrorKind::from_code(&service_error.code).ok_or_else(|| {
                malformed(format!(
                    "local daemon returned unknown error code {:?}",
                    service_error.code
                ))
            })?;
            return Err(DaemonError::new(kind, service_error.message));
        }
        if frame.kind != response_kind {
            return Err(malformed(format!(
                "expected {response_kind:?}, got {:?}",
                frame.kind
            )));
        }
        Ok(frame)
    }
}

#[cfg(not(unix))]
impl LocalClient {
    /// Returns the M3 platform limitation on non-Unix targets.
    pub async fn readiness(&self) -> Result<DaemonReadiness, DaemonError> {
        Err(unsupported())
    }

    /// Returns the M3 platform limitation on non-Unix targets.
    pub async fn status(&self) -> Result<DaemonStatus, DaemonError> {
        Err(unsupported())
    }

    /// Returns the M3 platform limitation on non-Unix targets.
    pub async fn validate_setup(
        &self,
        _requested: &ValidatedConfig,
    ) -> Result<ValidatedSetupStatus, DaemonError> {
        Err(unsupported())
    }

    /// Returns the M3 platform limitation on non-Unix targets.
    pub async fn stop(&self, _force: bool) -> Result<SessionImpact, DaemonError> {
        Err(unsupported())
    }

    /// Returns the M3 platform limitation on non-Unix targets.
    pub async fn update_preflight(&self) -> Result<SessionImpact, DaemonError> {
        Err(unsupported())
    }
}

fn decode_response<Message>(frame: &DecodedFrame) -> Result<Message, DaemonError>
where
    Message: prost::Message + Default,
{
    frame.decode_message(frame.kind).map_err(protocol_error)
}

fn protocol_status(protocol: Option<v1::ProtocolVersion>) -> Result<ProtocolStatus, DaemonError> {
    let protocol = protocol.ok_or_else(|| malformed("local response omitted protocol"))?;
    Ok(ProtocolStatus {
        wire_major: protocol.wire_major,
        state_schema: protocol.state_schema,
        capabilities: protocol.capabilities,
    })
}

fn connect_error(error: std::io::Error) -> DaemonError {
    let kind = match error.kind() {
        std::io::ErrorKind::PermissionDenied => DomainErrorKind::PermissionMismatch,
        _ => DomainErrorKind::DaemonStopped,
    };
    DaemonError::new(kind, format!("local daemon is unavailable: {error}"))
}

fn daemon_io(operation: &str, error: std::io::Error) -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DaemonStopped,
        format!("{operation}: {error}"),
    )
}

fn malformed(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

#[cfg(not(unix))]
fn unsupported() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::UnsupportedPlatform,
        "local daemon IPC is Unix-only in M3",
    )
}
