//! Bounded same-UID listener, first-frame ingress, and server dispatch.
use crate::{error::DaemonError, service::DaemonService};
#[cfg(unix)]
use crate::{
    service::{ServiceReply, protocol_error},
    session_wire::{SessionWireLimits, SessionWireServer, finish_unary, read_first},
};
#[cfg(unix)]
use iroh::SecretKey;
use std::sync::Arc;
#[cfg(unix)]
use std::time::{Duration, Instant};
#[cfg(unix)]
use tokio::{
    io::AsyncWriteExt,
    sync::{Semaphore, mpsc},
    task::JoinSet,
};
use zterm_core::DomainErrorKind;
#[cfg(unix)]
use zterm_core::{AttachmentId, DeviceId, ResourceLimits};
#[cfg(unix)]
use zterm_proto::{DecodedFrame, WireKind, v2};
#[cfg(unix)]
const DEFAULT_DEADLINE: Duration = Duration::from_secs(5);
#[cfg(unix)]
const DRAIN_GRACE: Duration = Duration::from_secs(30);

// Existing low-level callers retain their entry points during the module move.
#[cfg(unix)]
pub use crate::client::{
    LocalAttachmentEvent, LocalPairingClient, LocalTakeoverRetryToken,
    SessionClient as LocalAttachmentClient,
};
pub use crate::client::{LocalClient, LocalDeviceClient};

/// Fixed production limits with a reduced test constructor for deadline evidence.
#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
pub struct LocalIpcLimits {
    initial_read_timeout: Duration,
    default_request_deadline: Duration,
    maximum_request_deadline: Duration,
    maximum_connections: usize,
    injected_accept_failures: usize,
    injected_accept_after: usize,
    injected_fatal_accept_failures: usize,
    injected_fatal_accept_after: usize,
}

#[cfg(unix)]
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
            injected_accept_failures: 0,
            injected_accept_after: 0,
            injected_fatal_accept_failures: 0,
            injected_fatal_accept_after: 0,
        }
    }
}

#[cfg(unix)]
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
            injected_accept_failures: 0,
            injected_accept_after: 0,
            injected_fatal_accept_failures: 0,
            injected_fatal_accept_after: 0,
        }
    }

    /// Injects recoverable listener accept failures for lifecycle tests.
    #[doc(hidden)]
    #[must_use]
    pub const fn with_accept_failures_for_test(mut self, failures: usize) -> Self {
        self.injected_accept_failures = failures;
        self
    }

    /// Injects failures only after a number of successful accepts.
    #[doc(hidden)]
    #[must_use]
    pub const fn with_accept_failure_after_for_test(mut self, successful_accepts: usize) -> Self {
        self.injected_accept_failures = 1;
        self.injected_accept_after = successful_accepts;
        self
    }

    /// Injects one fatal accept failure after the requested successful accepts.
    #[doc(hidden)]
    #[must_use]
    pub const fn with_fatal_accept_failure_after_for_test(
        mut self,
        successful_accepts: usize,
    ) -> Self {
        self.injected_fatal_accept_failures = 1;
        self.injected_fatal_accept_after = successful_accepts;
        self
    }

    pub(crate) const fn without_accept_failure_injection(mut self) -> Self {
        self.injected_accept_failures = 0;
        self.injected_fatal_accept_failures = 0;
        self
    }

    fn request_deadline(self, requested_ms: u32) -> Duration {
        if requested_ms == 0 {
            self.default_request_deadline
        } else {
            Duration::from_millis(u64::from(requested_ms)).min(self.maximum_request_deadline)
        }
    }

    fn session_wire_limits(self) -> SessionWireLimits {
        SessionWireLimits::new(
            self.default_request_deadline,
            self.maximum_request_deadline,
            DEFAULT_DEADLINE,
        )
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
    serve_local_inner(
        listener,
        expected_uid,
        service,
        limits,
        #[cfg(test)]
        None,
    )
    .await
}

#[cfg(unix)]
async fn serve_local_inner(
    listener: std::os::unix::net::UnixListener,
    expected_uid: u32,
    service: Arc<DaemonService>,
    limits: LocalIpcLimits,
    #[cfg(test)] handler_reaped_for_test: Option<mpsc::UnboundedSender<usize>>,
) -> Result<(), DaemonError> {
    listener
        .set_nonblocking(true)
        .map_err(|error| daemon_io("configure local listener", error))?;
    let listener = tokio::net::UnixListener::from_std(listener)
        .map_err(|error| daemon_io("adopt local listener", error))?;
    let permits = Arc::new(Semaphore::new(limits.maximum_connections));
    let (stop_sender, mut stop_receiver) = mpsc::unbounded_channel();
    let mut handlers = JoinSet::new();
    let mut injected_accept_failures = limits.injected_accept_failures;
    let mut injected_fatal_accept_failures = limits.injected_fatal_accept_failures;
    let mut successful_accepts = 0_usize;
    let mut fatal_error = None;

    loop {
        tokio::select! {
            biased;
            stop = stop_receiver.recv() => {
                if stop.is_some() {
                    break;
                }
            }
            joined = handlers.join_next(), if !handlers.is_empty() => {
                observe_local_handler_completion(joined);
                #[cfg(test)]
                if let Some(observer) = &handler_reaped_for_test {
                    let _ = observer.send(handlers.len());
                }
            }
            accepted = async {
                if injected_fatal_accept_failures > 0
                    && successful_accepts >= limits.injected_fatal_accept_after
                {
                    injected_fatal_accept_failures -= 1;
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "injected fatal local listener accept failure",
                    ))
                } else if injected_accept_failures > 0
                    && successful_accepts >= limits.injected_accept_after
                {
                    injected_accept_failures -= 1;
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "injected recoverable local listener accept failure",
                    ))
                } else {
                    listener.accept().await
                }
            } => {
                let (stream, _) = match accepted {
                    Ok(accepted) => {
                        successful_accepts = successful_accepts.saturating_add(1);
                        accepted
                    }
                    Err(error) => {
                        if recoverable_accept_error(&error) {
                            // Per-connection accept failures do not transfer or
                            // invalidate daemon/session ownership.
                            tracing::warn!(
                                error_kind = ?error.kind(),
                                "local listener accept failed; retrying"
                            );
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            continue;
                        }
                        fatal_error = Some(daemon_io("accept local connection", error));
                        break;
                    }
                };
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
    if fatal_error.is_some() {
        handlers.abort_all();
        while handlers.join_next().await.is_some() {}
    } else {
        let drain = async {
            while let Some(joined) = handlers.join_next().await {
                observe_local_handler_completion(Some(joined));
            }
        };
        if tokio::time::timeout(DRAIN_GRACE, drain).await.is_err() {
            handlers.abort_all();
        }
    }
    fatal_error.map_or(Ok(()), Err)
}

#[cfg(unix)]
fn observe_local_handler_completion(joined: Option<Result<(), tokio::task::JoinError>>) {
    match joined {
        Some(Ok(())) => {}
        Some(Err(error)) => tracing::warn!(
            cancelled = error.is_cancelled(),
            panicked = error.is_panic(),
            "local connection handler task ended unexpectedly"
        ),
        None => {
            tracing::warn!("local connection handler ownership ended without a join result");
        }
    }
}

#[cfg(unix)]
fn recoverable_accept_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::TimedOut
    )
}

#[cfg(not(unix))]
/// Returns the current platform limitation on non-Unix targets.
pub async fn serve_local(
    _listener: (),
    _expected_uid: u32,
    _service: Arc<DaemonService>,
) -> Result<(), DaemonError> {
    Err(DaemonError::new(
        DomainErrorKind::UnsupportedPlatform,
        "local daemon IPC is Unix-only in the current milestone",
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
    let first =
        match tokio::time::timeout(limits.initial_read_timeout, read_first(&mut stream)).await {
            Ok(Ok(first)) => first,
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
    if first.frame.kind == WireKind::LocalSessionTunnelOpenRequest {
        let deadline = limits.request_deadline(first.frame.deadline_ms);
        let absolute_deadline = started + deadline;
        let result = match service.remote_sessions() {
            Some(remote) => {
                remote
                    .serve_tunnel(
                        stream,
                        first,
                        limits.session_wire_limits(),
                        absolute_deadline,
                    )
                    .await
            }
            None => {
                let error = DaemonError::new(
                    DomainErrorKind::TransportUnavailable,
                    "remote Session transport is not composed into this daemon",
                );
                let _ = write_error(&mut stream, first.frame.request_id, &error).await;
                Err(error)
            }
        };
        if let Err(error) = result {
            tracing::debug!(
                error_kind = error.kind().code(),
                "local remote-Session tunnel closed"
            );
        }
        return;
    }
    if first.frame.kind == WireKind::TerminalAttachRequest {
        let deadline = limits.request_deadline(first.frame.deadline_ms);
        let absolute_deadline = started + deadline;
        let target = match terminal_attach_target(&first.frame) {
            Ok(target) => target,
            Err(error) => {
                let _ = write_error(&mut stream, first.frame.request_id, &error).await;
                return;
            }
        };
        let local_view_id = local_view_id();
        if target.is_some() {
            let error = malformed(
                "remote terminal attachment requires a local Session tunnel Open request",
            );
            let _ = write_error(&mut stream, first.frame.request_id, &error).await;
            return;
        }
        let result = SessionWireServer::new(service.sessions().clone())
            .handle_local_attachment(
                stream,
                first,
                local_view_id,
                limits.session_wire_limits(),
                absolute_deadline,
            )
            .await;
        if let Err(error) = result {
            tracing::debug!(
                error_kind = error.kind().code(),
                "local terminal attachment closed"
            );
        }
        return;
    }
    // Only copy non-sensitive routing metadata. The decoded frame itself is
    // moved through strict unary EOF validation and then into the dispatcher;
    // pair/device request payloads are never cloned by the generic classifier.
    let request_id = first.frame.request_id;
    let deadline = limits.request_deadline(first.frame.deadline_ms);
    let absolute_deadline = started + deadline;
    let remaining = deadline.saturating_sub(started.elapsed());
    let unary_finished = tokio::time::timeout(remaining, finish_unary(&mut stream, first)).await;
    let frame = match unary_finished.unwrap_or_else(|_| {
        Err(DaemonError::new(
            DomainErrorKind::DeadlineExceeded,
            "local unary request did not finish before its deadline",
        ))
    }) {
        Ok(frame) => frame,
        Err(error) => {
            let _ = write_error(&mut stream, request_id, &error).await;
            return;
        }
    };
    let remaining = deadline.saturating_sub(started.elapsed());
    let session_wire = SessionWireServer::new(service.sessions().clone());
    let reply = match tokio::time::timeout(remaining, async {
        if SessionWireServer::handles_unary(frame.kind) {
            session_wire
                .dispatch_local_unary_until(frame, absolute_deadline)
                .await
        } else {
            service.dispatch_until(frame, absolute_deadline).await
        }
    })
    .await
    {
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
fn local_view_id() -> AttachmentId {
    let secret = SecretKey::generate().to_bytes();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&secret[..16]);
    AttachmentId::from_array(bytes)
}

#[cfg(unix)]
fn terminal_attach_target(frame: &DecodedFrame) -> Result<Option<DeviceId>, DaemonError> {
    let request: v2::TerminalAttachRequest = frame
        .decode_message(WireKind::TerminalAttachRequest)
        .map_err(protocol_error)?;
    match request.target.and_then(|target| target.target) {
        Some(v2::target_selector::Target::Local(true)) => Ok(None),
        Some(v2::target_selector::Target::Device(device)) => {
            device.try_into().map(Some).map_err(protocol_error)
        }
        _ => Err(malformed(
            "terminal attach requires either target.local=true or one full device target",
        )),
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
fn daemon_io(operation: &str, error: std::io::Error) -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DaemonStopped,
        format!("{operation}: {error}"),
    )
}

#[cfg(unix)]
fn malformed(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

#[cfg(not(unix))]
fn unsupported() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::UnsupportedPlatform,
        "local daemon IPC is Unix-only in the current milestone",
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::client::resolved_target_wire;
    use crate::device_directory::ResolvedSessionTarget;
    use zterm_proto::{FrameDecoder, encode_message};
    #[tokio::test]
    async fn completed_handlers_are_reaped_before_subsequent_connection_churn() {
        let temporary = tempfile::tempdir().expect("temporary handler-reap fixture");
        let socket = temporary.path().join("handler-reap.sock");
        let listener =
            std::os::unix::net::UnixListener::bind(&socket).expect("bind handler-reap listener");
        let config = crate::config::validate_setup_input(
            "handler-reap",
            crate::config::ValidatedInfrastructure::OfficialN0,
        )
        .expect("valid handler-reap setup");
        let setup = crate::bootstrap::BootstrapResult {
            device_id: DeviceId::from_array([0x71; DeviceId::LENGTH]),
            endpoint_id: "handler-reap-endpoint".to_owned(),
            config,
        };
        let service = Arc::new(DaemonService::with_started_at(setup, 1));
        let (reaped, mut reaped_observations) = mpsc::unbounded_channel();
        let server = tokio::spawn(serve_local_inner(
            listener,
            nix::unistd::Uid::effective().as_raw(),
            service,
            LocalIpcLimits::for_test(Duration::from_secs(2)),
            Some(reaped),
        ));
        let client = LocalClient::new(&socket);

        for request_index in 0..4 {
            client
                .readiness()
                .await
                .expect("readiness handler completes");
            let retained = tokio::time::timeout(Duration::from_secs(2), reaped_observations.recv())
                .await
                .expect("completed handler is observed without polling")
                .expect("handler-reap observer remains installed");
            assert_eq!(
                retained, 0,
                "request {request_index} returned JoinSet ownership to baseline before the next request",
            );
        }

        assert!(
            client
                .stop(false)
                .await
                .expect("stop empty fixture")
                .stopping
        );
        server
            .await
            .expect("handler-reap server task")
            .expect("handler-reap server result");
    }

    #[test]
    fn local_attachment_views_are_fresh_fixed_width_ids() {
        let first = local_view_id();
        let second = local_view_id();
        assert_ne!(first, second);
        assert_eq!(first.to_bytes().len(), AttachmentId::LENGTH);
    }

    #[test]
    fn terminal_first_frame_router_separates_local_and_exact_device_targets() {
        let target = DeviceId::from_array([0x91; DeviceId::LENGTH]);
        let local = decoded_attach_target(v2::TargetSelector {
            target: Some(v2::target_selector::Target::Local(true)),
        });
        assert_eq!(
            terminal_attach_target(&local).expect("local target routes"),
            None
        );

        let remote =
            decoded_attach_target(resolved_target_wire(ResolvedSessionTarget::device(target)));
        assert_eq!(
            terminal_attach_target(&remote).expect("device target routes"),
            Some(target)
        );

        let false_local = decoded_attach_target(v2::TargetSelector {
            target: Some(v2::target_selector::Target::Local(false)),
        });
        assert_eq!(
            terminal_attach_target(&false_local)
                .expect_err("false local selector is not a routing target")
                .kind(),
            DomainErrorKind::MalformedFrame
        );
    }

    fn decoded_attach_target(target: v2::TargetSelector) -> DecodedFrame {
        let bytes = encode_message(
            WireKind::TerminalAttachRequest,
            1,
            0,
            &v2::TerminalAttachRequest {
                target: Some(target),
                session_id: None,
                takeover: false,
                session_name: String::new(),
                create_main: true,
                viewport: None,
                resume_view_id: None,
                known_revision: None,
            },
        )
        .expect("bounded attach routing fixture");
        let mut decoder = FrameDecoder::new();
        let mut frames = decoder.feed(&bytes).expect("decode attach routing fixture");
        decoder.finish().expect("complete attach routing fixture");
        assert_eq!(frames.len(), 1);
        frames.remove(0)
    }
}
