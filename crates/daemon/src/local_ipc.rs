//! Bounded same-UID unary and terminal IPC over Unix-domain sockets.

#[cfg(unix)]
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(unix)]
use std::sync::Mutex as StdMutex;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use iroh::SecretKey;
#[cfg(unix)]
use ring::rand::{SecureRandom, SystemRandom};
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::sync::{Mutex as AsyncMutex, Semaphore, mpsc};
#[cfg(unix)]
use tokio::task::JoinSet;
#[cfg(unix)]
use zeroize::{Zeroize, Zeroizing};
#[cfg(unix)]
use zterm_core::terminal::{
    MAX_HISTORY_PAGE_ROWS, TerminalHistoryCursor, TerminalHistoryDirection,
    TerminalHistoryWindowAnchor, TerminalHistoryWindowQuery, TerminalScrollAction, TerminalSize,
    TerminalViewportDisposition,
};
#[cfg(unix)]
use zterm_core::{
    AttachmentId, DEFAULT_PAIR_TTL_SECONDS, EphemeralOperationId, OperationId, OperationLease,
    PairFingerprint, ResourceLimits, Revision, SessionSelector,
};
use zterm_core::{DeviceAlias, DeviceId, DeviceSummary, DomainErrorKind, SessionId, SessionName};
#[cfg(unix)]
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v1};

use crate::config::ValidatedConfig;
use crate::device_directory::ResolvedSessionTarget;
use crate::error::DaemonError;
#[cfg(unix)]
use crate::network::{AddressServiceState, NetworkDiagnostic, NetworkObservation, NetworkState};
#[cfg(unix)]
use crate::pairing::PairTicketText;
#[cfg(unix)]
use crate::remote_session::{
    SessionUnaryResponseStatus, session_summary_from_wire, validate_session_unary_response,
};
use crate::service::{
    DaemonReadiness, DaemonService, DaemonStatus, SessionImpact, ValidatedSetupStatus,
};
#[cfg(unix)]
use crate::service::{ProtocolStatus, ServiceReply, protocol_error};
#[cfg(unix)]
use crate::session_wire::{SessionWireLimits, SessionWireServer, finish_unary, read_first};

#[cfg(unix)]
const DEFAULT_DEADLINE: Duration = Duration::from_secs(5);
#[cfg(unix)]
const PAIRING_DEADLINE: Duration = Duration::from_secs(15);
#[cfg(unix)]
const DRAIN_GRACE: Duration = Duration::from_secs(30);
#[cfg(unix)]
const MAX_MUTATION_TARGETS_PER_CLIENT: usize = 64;
#[cfg(unix)]
const ATTACHMENT_COMMAND_STREAM_CLOSED: &str = "local terminal attachment command stream closed";
#[cfg(unix)]
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
        let result = if let Some(target) = target {
            match service.remote_sessions() {
                Some(remote) => {
                    remote
                        .bridge_attachment(
                            target,
                            local_view_id,
                            stream,
                            first,
                            limits.session_wire_limits(),
                            absolute_deadline,
                        )
                        .await
                }
                None => Err(DaemonError::new(
                    DomainErrorKind::TransportUnavailable,
                    "remote Session transport is not composed into this daemon",
                )),
            }
        } else {
            SessionWireServer::new(service.sessions().clone())
                .handle_local_attachment(
                    stream,
                    first,
                    local_view_id,
                    limits.session_wire_limits(),
                    absolute_deadline,
                )
                .await
        };
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
    let request: v1::TerminalAttachRequest = frame
        .decode_message(WireKind::TerminalAttachRequest)
        .map_err(protocol_error)?;
    match request.target.and_then(|target| target.target) {
        Some(v1::target_selector::Target::Local(true)) => Ok(None),
        Some(v1::target_selector::Target::Device(device)) => {
            device.try_into().map(Some).map_err(protocol_error)
        }
        _ => Err(malformed(
            "terminal attach requires either target.local=true or one full device target",
        )),
    }
}

/// One typed server message received on a local terminal attachment.
#[cfg(unix)]
#[derive(Clone, PartialEq)]
#[doc(hidden)]
pub enum LocalAttachmentEvent {
    /// Latest daemon-owned connection state for a remote desired view.
    TransportState(v1::TerminalTransportStateEvent),
    /// Address-free selected path and RTT for a remote desired view.
    ConnectionStatus(v1::TerminalConnectionStatusEvent),
    /// A full host-authoritative replacement state.
    Snapshot(v1::TerminalSnapshot),
    /// A merged revision update from the acknowledged checkpoint.
    Delta(v1::TerminalDelta),
    /// One correlated bounded page from daemon-authoritative history.
    HistoryPage(v1::TerminalHistoryPage),
    /// One correlated complete attachment-local semantic viewport outcome.
    ViewportFrame(v1::TerminalViewportFrame),
    /// One correlated stateless bounded history-window outcome.
    HistoryWindowFrame(v1::TerminalHistoryWindowFrame),
    /// The following snapshot must replace the client state atomically.
    SyncRequired(v1::TerminalSyncRequired),
    /// A prepared takeover committed successfully.
    Takeover(crate::session::SessionSummary),
    /// Another attachment replaced this controller.
    LeaseLost(v1::TerminalLeaseLost),
    /// The underlying session and PTY ended.
    SessionEnded(v1::TerminalSessionEnded),
}

#[cfg(unix)]
impl fmt::Debug for LocalAttachmentEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TransportState(state) => formatter
                .debug_struct("TransportState")
                .field("state", &state.state)
                .finish_non_exhaustive(),
            Self::ConnectionStatus(status) => formatter
                .debug_struct("ConnectionStatus")
                .field("path", &status.path)
                .field("rtt_ms", &status.rtt_ms)
                .finish_non_exhaustive(),
            Self::Snapshot(snapshot) => formatter
                .debug_struct("Snapshot")
                .field("revision", &snapshot.revision)
                .field("screen_ansi_len", &snapshot.screen_ansi.len())
                .field(
                    "recent_history_ansi_len",
                    &snapshot.recent_history_ansi.len(),
                )
                .finish_non_exhaustive(),
            Self::Delta(delta) => formatter
                .debug_struct("Delta")
                .field("from_revision", &delta.from_revision)
                .field("to_revision", &delta.to_revision)
                .field("ansi_len", &delta.ansi.len())
                .finish_non_exhaustive(),
            Self::HistoryPage(page) => formatter
                .debug_struct("HistoryPage")
                .field("outcome", &page.outcome)
                .field("row_count", &page.rows.len())
                .finish_non_exhaustive(),
            Self::ViewportFrame(frame) => formatter
                .debug_struct("ViewportFrame")
                .field("outcome", &frame.outcome)
                .field("row_count", &frame.rows.len())
                .finish_non_exhaustive(),
            Self::HistoryWindowFrame(frame) => formatter
                .debug_struct("HistoryWindowFrame")
                .field("outcome", &frame.outcome)
                .field("row_count", &frame.ansi_rows.len())
                .finish_non_exhaustive(),
            Self::SyncRequired(required) => formatter
                .debug_struct("SyncRequired")
                .field("latest_revision", &required.latest_revision)
                .finish_non_exhaustive(),
            Self::Takeover(summary) => formatter.debug_tuple("Takeover").field(summary).finish(),
            Self::LeaseLost(lost) => formatter
                .debug_struct("LeaseLost")
                .field("generation", &lost.generation)
                .finish_non_exhaustive(),
            Self::SessionEnded(ended) => formatter
                .debug_struct("SessionEnded")
                .field("reason", &ended.reason)
                .field("exit_code", &ended.exit_code)
                .field("has_signal", &!ended.signal.is_empty())
                .finish_non_exhaustive(),
        }
    }
}

/// Opaque same-daemon retry token for one takeover whose response was lost.
///
/// It is intentionally process-memory only in M4. Callers must export and
/// retain it explicitly if they need fresh-process ambiguity recovery.
#[cfg(unix)]
#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct LocalTakeoverRetryToken {
    operation_id: OperationId,
    session_id: SessionId,
}

#[cfg(unix)]
impl fmt::Debug for LocalTakeoverRetryToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LocalTakeoverRetryToken([REDACTED])")
    }
}

/// Real same-UID duplex socket adapter used before the final raw-terminal UI exists.
#[cfg(unix)]
#[doc(hidden)]
pub struct LocalAttachmentClient {
    stream: tokio::net::UnixStream,
    decoder: FrameDecoder,
    queued: VecDeque<DecodedFrame>,
    deferred: VecDeque<DecodedFrame>,
    session_id: SessionId,
    attachment_id: AttachmentId,
    target: ResolvedSessionTarget,
    initial_snapshot: v1::TerminalSnapshot,
    terminal_rows: u32,
    next_request_id: u64,
    operation_lease: Option<OperationLease>,
    next_operation_sequence: u64,
    pending_takeover_request_id: Option<u64>,
    pending_history_request_id: Option<u64>,
    pending_viewport_request_id: Option<u64>,
    pending_history_window: Option<(u64, TerminalHistoryWindowQuery)>,
}

#[cfg(unix)]
impl fmt::Debug for LocalAttachmentClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalAttachmentClient")
            .field("session_id", &self.session_id)
            .field("attachment_id", &self.attachment_id)
            .field("initial_revision", &self.initial_snapshot.revision)
            .field("queued_frames", &self.queued.len())
            .field("deferred_frames", &self.deferred.len())
            .field("has_operation_lease", &self.operation_lease.is_some())
            .field(
                "has_pending_takeover",
                &self.pending_takeover_request_id.is_some(),
            )
            .field(
                "has_pending_history",
                &self.pending_history_request_id.is_some(),
            )
            .field(
                "has_pending_viewport",
                &self.pending_viewport_request_id.is_some(),
            )
            .field(
                "has_pending_history_window",
                &self.pending_history_window.is_some(),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(unix)]
impl LocalAttachmentClient {
    /// Opens one already-resolved local or remote view for the high-level
    /// command runtime without exposing the socket or target token to the CLI.
    pub(crate) async fn connect_resolved(
        socket: impl AsRef<Path>,
        target: ResolvedSessionTarget,
        selector: Option<SessionSelector>,
        create_main: bool,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            socket.as_ref(),
            target,
            selector,
            create_main,
            takeover,
            viewport,
        )
        .await
    }

    /// Attaches to the daemon-lifetime default `main` session, creating it if absent.
    pub async fn connect_main(
        socket: impl AsRef<Path>,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            socket.as_ref(),
            ResolvedSessionTarget::local(),
            None,
            true,
            false,
            viewport,
        )
        .await
    }

    /// Attaches to an existing session selected by stable ID or exact name.
    pub async fn connect_session(
        socket: impl AsRef<Path>,
        selector: SessionSelector,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            socket.as_ref(),
            ResolvedSessionTarget::local(),
            Some(selector),
            false,
            takeover,
            viewport,
        )
        .await
    }

    /// Opens one daemon-bridged remote default view without exposing Iroh.
    #[doc(hidden)]
    pub async fn connect_remote_main(
        socket: impl AsRef<Path>,
        target: DeviceId,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            socket.as_ref(),
            ResolvedSessionTarget::device(target),
            None,
            true,
            false,
            viewport,
        )
        .await
    }

    /// Opens one daemon-bridged remote named/ID view without exposing Iroh.
    #[doc(hidden)]
    pub async fn connect_remote_session(
        socket: impl AsRef<Path>,
        target: DeviceId,
        selector: SessionSelector,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            socket.as_ref(),
            ResolvedSessionTarget::device(target),
            Some(selector),
            false,
            takeover,
            viewport,
        )
        .await
    }

    async fn connect_inner(
        socket: &Path,
        target: ResolvedSessionTarget,
        selector: Option<SessionSelector>,
        create_main: bool,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        let (session_id, session_name) = match selector {
            Some(SessionSelector::Id(session_id)) => (Some(session_id.into()), String::new()),
            Some(SessionSelector::Name(name)) => (None, name.to_string()),
            None => (None, String::new()),
        };
        let request_id = 1;
        let bytes = encode_message(
            WireKind::TerminalAttachRequest,
            request_id,
            u32::try_from(DEFAULT_DEADLINE.as_millis()).unwrap_or(u32::MAX),
            &v1::TerminalAttachRequest {
                target: Some(resolved_target_wire(target)),
                session_id,
                takeover,
                session_name,
                create_main,
                viewport: viewport.map(Into::into),
                resume_view_id: None,
                known_revision: None,
            },
        )
        .map_err(protocol_error)?;
        let mut stream = tokio::net::UnixStream::connect(socket)
            .await
            .map_err(connect_error)?;
        if let Err(error) = stream.write_all(&bytes).await {
            let error = daemon_io("write local attach request", error);
            return Err(if create_main {
                create_main_outcome_unknown()
            } else {
                error
            });
        }

        // The outer result owns post-write ambiguity. The inner result is
        // reserved for a decoded, correlated ServiceError, which is already a
        // definitive result and must retain its exact domain category.
        let response = tokio::time::timeout(DEFAULT_DEADLINE, async {
            let first = read_first(&mut stream).await?;
            let mut decoder = first.decoder;
            let mut queued = first.queued;
            let mut current = Some(first.frame);
            let mut pre_snapshot_states = Vec::new();
            let initial_snapshot = loop {
                let frame = match current.take() {
                    Some(frame) => frame,
                    None => read_frame_parts(&mut stream, &mut decoder, &mut queued).await?,
                };
                if frame.kind == WireKind::ServiceErrorResponse {
                    if frame.request_id != request_id {
                        return Err(malformed("initial terminal error correlation mismatch"));
                    }
                    return Ok(Err(service_error(&frame)?));
                }
                if frame.kind == WireKind::TerminalTransportStateEvent {
                    let state: v1::TerminalTransportStateEvent = frame
                        .decode_message(WireKind::TerminalTransportStateEvent)
                        .map_err(protocol_error)?;
                    v1::TerminalTransportState::try_from(state.state)
                        .map_err(|_| malformed("unknown terminal transport state"))?;
                    pre_snapshot_states.push(state);
                    continue;
                }
                if frame.kind != WireKind::TerminalSnapshot || frame.request_id != request_id {
                    return Err(malformed("initial terminal snapshot correlation mismatch"));
                }
                break frame
                    .decode_message(WireKind::TerminalSnapshot)
                    .map_err(protocol_error)?;
            };
            let session_id = required_snapshot_session_id(&initial_snapshot)?;
            let attachment_id = required_snapshot_attachment_id(&initial_snapshot)?;
            validate_snapshot_viewport(&initial_snapshot)?;
            let terminal_rows = initial_snapshot.rows;
            for state in &pre_snapshot_states {
                let state_attachment: AttachmentId = state
                    .attachment_id
                    .clone()
                    .ok_or_else(|| malformed("transport state omitted attachment_id"))?
                    .try_into()
                    .map_err(protocol_error)?;
                if state_attachment != attachment_id {
                    return Err(malformed("transport state attachment_id mismatch"));
                }
            }
            Ok(Ok(Self {
                stream,
                decoder,
                queued,
                deferred: VecDeque::new(),
                session_id,
                attachment_id,
                target,
                initial_snapshot,
                terminal_rows,
                next_request_id: request_id + 1,
                operation_lease: None,
                next_operation_sequence: 1,
                pending_takeover_request_id: None,
                pending_history_request_id: None,
                pending_viewport_request_id: None,
                pending_history_window: None,
            }))
        })
        .await;

        match response {
            Ok(Ok(result)) => result,
            Ok(Err(_)) if create_main => Err(create_main_outcome_unknown()),
            Ok(Err(error)) => Err(error),
            Err(_) if create_main => Err(create_main_outcome_unknown()),
            Err(_) => Err(DaemonError::new(
                DomainErrorKind::DeadlineExceeded,
                "timed out waiting for initial terminal snapshot",
            )),
        }
    }

    /// Returns the attached daemon-lifetime session identity.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns this socket view's attachment identity.
    #[must_use]
    pub const fn attachment_id(&self) -> AttachmentId {
        self.attachment_id
    }

    /// Whether this local socket is backed by a daemon-owned remote bridge.
    #[must_use]
    pub const fn is_remote(&self) -> bool {
        self.target.device_id().is_some()
    }

    /// Returns the initial full state which must be acknowledged before input.
    #[must_use]
    pub const fn initial_snapshot(&self) -> &v1::TerminalSnapshot {
        &self.initial_snapshot
    }

    #[cfg(test)]
    pub(crate) fn terminal_driver_test_pair(
        target: ResolvedSessionTarget,
        session_id: SessionId,
        attachment_id: AttachmentId,
    ) -> (Self, tokio::net::UnixStream) {
        let (stream, peer) =
            tokio::net::UnixStream::pair().expect("test-private terminal driver Unix stream pair");
        (
            Self {
                stream,
                decoder: FrameDecoder::new(),
                queued: VecDeque::new(),
                deferred: VecDeque::new(),
                session_id,
                attachment_id,
                target,
                initial_snapshot: v1::TerminalSnapshot {
                    session_id: Some(session_id.into()),
                    attachment_id: Some(attachment_id.into()),
                    revision: 1,
                    rows: 24,
                    columns: 80,
                    screen_ansi: Vec::new(),
                    recent_history_ansi: Vec::new(),
                    active_screen: v1::TerminalActiveScreen::Main as i32,
                    modes: Some(v1::TerminalModes::default()),
                    scroll_metrics: None,
                },
                terminal_rows: 24,
                next_request_id: 2,
                operation_lease: None,
                next_operation_sequence: 1,
                pending_takeover_request_id: None,
                pending_history_request_id: None,
                pending_viewport_request_id: None,
                pending_history_window: None,
            },
            peer,
        )
    }

    /// Atomically acknowledges the exact full snapshot revision.
    pub async fn snapshot_applied(&mut self, revision: Revision) -> Result<(), DaemonError> {
        self.send(
            WireKind::TerminalSnapshotApplied,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(self.attachment_id.into()),
                revision: revision.get(),
            },
        )
        .await
        .map(|_| ())
    }

    /// Sends controller input without waiting for a redundant success ACK.
    pub async fn write_input(&mut self, bytes: Vec<u8>) -> Result<(), DaemonError> {
        self.send(
            WireKind::TerminalInput,
            &v1::TerminalInput {
                operation_id: None,
                attachment_id: Some(self.attachment_id.into()),
                bytes,
            },
        )
        .await
        .map(|_| ())
    }

    /// Requests one validated native/model viewport change.
    pub async fn resize(
        &mut self,
        size: zterm_core::terminal::TerminalSize,
    ) -> Result<(), DaemonError> {
        self.send(
            WireKind::TerminalResize,
            &v1::TerminalResize {
                operation_id: None,
                attachment_id: Some(self.attachment_id.into()),
                rows: u32::from(size.rows),
                columns: u32::from(size.columns),
            },
        )
        .await
        .map(|_| ())
    }

    /// Discards the client baseline and requests a fresh snapshot.
    pub async fn request_sync(&mut self, known_revision: Revision) -> Result<(), DaemonError> {
        self.send(
            WireKind::TerminalSyncRequest,
            &v1::TerminalSyncRequest {
                attachment_id: Some(self.attachment_id.into()),
                known_revision: known_revision.get(),
            },
        )
        .await
        .map(|_| ())
    }

    /// Requests one bounded page from daemon-authoritative main-screen
    /// history. Exactly one page request may be outstanding per view.
    pub(crate) async fn request_history(
        &mut self,
        direction: TerminalHistoryDirection,
        cursor: Option<TerminalHistoryCursor>,
        maximum_rows: usize,
    ) -> Result<(), DaemonError> {
        if self.pending_history_request_id.is_some() {
            return Err(resource_error(
                "a terminal history page response is already pending",
            ));
        }
        if maximum_rows == 0 || maximum_rows > MAX_HISTORY_PAGE_ROWS {
            return Err(resource_error(
                "terminal history page bound is outside the allowed range",
            ));
        }
        let direction = match direction {
            TerminalHistoryDirection::Newest => v1::TerminalHistoryDirection::Newest,
            TerminalHistoryDirection::Older => v1::TerminalHistoryDirection::Older,
            TerminalHistoryDirection::Newer => v1::TerminalHistoryDirection::Newer,
        };
        let request_id = self
            .send(
                WireKind::TerminalHistoryRequest,
                &v1::TerminalHistoryRequest {
                    attachment_id: Some(self.attachment_id.into()),
                    direction: direction as i32,
                    cursor: cursor.map(Into::into),
                    maximum_rows: u32::try_from(maximum_rows).map_err(|_| {
                        resource_error("terminal history page bound is not representable")
                    })?,
                },
            )
            .await?;
        self.pending_history_request_id = Some(request_id);
        Ok(())
    }

    /// Requests one complete attachment-local semantic viewport outcome.
    /// Exactly one viewport request may be outstanding per view.
    pub(crate) async fn request_viewport(
        &mut self,
        action: TerminalScrollAction,
    ) -> Result<(), DaemonError> {
        if self.pending_viewport_request_id.is_some() {
            return Err(resource_error(
                "a terminal viewport response is already pending",
            ));
        }
        let action = match action {
            TerminalScrollAction::ScrollByLines(lines) => {
                v1::terminal_viewport_action::Action::ScrollByLines(lines)
            }
            TerminalScrollAction::ScrollToOffset(offset) => {
                v1::terminal_viewport_action::Action::ScrollToOffset(offset)
            }
        };
        let request_id = self
            .send(
                WireKind::TerminalViewportRequest,
                &v1::TerminalViewportRequest {
                    attachment_id: Some(self.attachment_id.into()),
                    action: Some(v1::TerminalViewportAction {
                        action: Some(action),
                    }),
                },
            )
            .await?;
        self.pending_viewport_request_id = Some(request_id);
        Ok(())
    }

    /// Requests one stateless bounded history window. Exactly one such request
    /// may be outstanding per view.
    pub(crate) async fn request_history_window(
        &mut self,
        query: TerminalHistoryWindowQuery,
    ) -> Result<(), DaemonError> {
        if self.pending_history_window.is_some() {
            return Err(resource_error(
                "a terminal history window response is already pending",
            ));
        }
        if !query.is_valid() {
            return Err(resource_error(
                "terminal history window query is outside the allowed range",
            ));
        }
        let request_id = self
            .send(
                WireKind::TerminalHistoryWindowRequest,
                &v1::TerminalHistoryWindowRequest {
                    attachment_id: Some(self.attachment_id.into()),
                    anchor: Some(query.anchor.into()),
                    target_offset_from_bottom: query.target_offset_from_bottom,
                    older_margin_rows: u32::from(query.older_margin_rows),
                    newer_margin_rows: u32::from(query.newer_margin_rows),
                },
            )
            .await?;
        self.pending_history_window = Some((request_id, query));
        Ok(())
    }

    /// Commits a previously prepared and acknowledged takeover attachment.
    pub async fn takeover(&mut self) -> Result<(), DaemonError> {
        self.begin_takeover().await.map(|_| ())
    }

    /// Sends a takeover and returns the opaque token required after ambiguous
    /// response loss.
    #[doc(hidden)]
    pub async fn begin_takeover(&mut self) -> Result<LocalTakeoverRetryToken, DaemonError> {
        if self.pending_takeover_request_id.is_some() {
            return Err(malformed("a takeover response is already pending"));
        }
        let operation_id = self.next_operation_id().await?;
        let receipt = LocalTakeoverRetryToken {
            operation_id,
            session_id: self.session_id,
        };
        self.pending_takeover_request_id = Some(self.send_takeover(operation_id).await?);
        Ok(receipt)
    }

    /// Continues an ambiguously completed takeover on a newly synchronized
    /// attachment without inventing a new logical operation.
    #[doc(hidden)]
    pub async fn retry_takeover(
        &mut self,
        token: LocalTakeoverRetryToken,
    ) -> Result<(), DaemonError> {
        if token.session_id != self.session_id {
            return Err(malformed("takeover retry token belongs to another session"));
        }
        if self.pending_takeover_request_id.is_some() {
            return Err(malformed("a takeover response is already pending"));
        }
        self.pending_takeover_request_id = Some(self.send_takeover(token.operation_id).await?);
        Ok(())
    }

    async fn send_takeover(&mut self, operation_id: OperationId) -> Result<u64, DaemonError> {
        self.send(
            WireKind::SessionTakeoverRequest,
            &v1::SessionTakeoverRequest {
                operation_id: Some(operation_id.into()),
                target: Some(resolved_target_wire(self.target)),
                session_id: Some(self.session_id.into()),
                attachment_id: Some(self.attachment_id.into()),
            },
        )
        .await
    }

    /// Reads one typed terminal event, bounded by the caller's deadline.
    pub async fn read_event(
        &mut self,
        deadline: Duration,
    ) -> Result<LocalAttachmentEvent, DaemonError> {
        tokio::time::timeout(deadline, self.read_next_event())
            .await
            .map_err(|_| {
                DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    "timed out waiting for local terminal event",
                )
            })?
    }

    /// Reads the next event without imposing a timeout on an active view.
    ///
    /// The owner may cancel and repoll this future: decoder and queued-byte
    /// state stay in this same client object.
    pub(crate) async fn read_next_event(&mut self) -> Result<LocalAttachmentEvent, DaemonError> {
        let frame = self.read_frame().await?;
        if frame.kind == WireKind::ServiceErrorResponse {
            let error = service_error(&frame)?;
            if self.pending_takeover_request_id == Some(frame.request_id) {
                self.pending_takeover_request_id = None;
            }
            if self.pending_history_request_id == Some(frame.request_id) {
                self.pending_history_request_id = None;
            }
            if self.pending_viewport_request_id == Some(frame.request_id) {
                self.pending_viewport_request_id = None;
            }
            if self
                .pending_history_window
                .is_some_and(|(request_id, _)| request_id == frame.request_id)
            {
                self.pending_history_window = None;
            }
            if error.kind() == DomainErrorKind::OperationOutcomeUnknown {
                self.operation_lease = None;
                self.next_operation_sequence = 1;
            }
            return Err(error);
        }
        match frame.kind {
            WireKind::TerminalTransportStateEvent => {
                let state: v1::TerminalTransportStateEvent = frame
                    .decode_message(WireKind::TerminalTransportStateEvent)
                    .map_err(protocol_error)?;
                self.require_attachment(state.attachment_id.clone())?;
                v1::TerminalTransportState::try_from(state.state)
                    .map_err(|_| malformed("unknown terminal transport state"))?;
                Ok(LocalAttachmentEvent::TransportState(state))
            }
            WireKind::TerminalConnectionStatusEvent => {
                let status: v1::TerminalConnectionStatusEvent = frame
                    .decode_message(WireKind::TerminalConnectionStatusEvent)
                    .map_err(protocol_error)?;
                self.require_attachment(status.attachment_id.clone())?;
                match v1::TerminalConnectionPath::try_from(status.path) {
                    Ok(v1::TerminalConnectionPath::Unknown)
                    | Ok(v1::TerminalConnectionPath::Direct)
                    | Ok(v1::TerminalConnectionPath::Relay) => {}
                    Ok(v1::TerminalConnectionPath::Unspecified) | Err(_) => {
                        return Err(malformed("unknown terminal connection path"));
                    }
                }
                Ok(LocalAttachmentEvent::ConnectionStatus(status))
            }
            WireKind::TerminalSnapshot => {
                let snapshot: v1::TerminalSnapshot = frame
                    .decode_message(WireKind::TerminalSnapshot)
                    .map_err(protocol_error)?;
                self.require_snapshot_identity(&snapshot)?;
                validate_snapshot_viewport(&snapshot)?;
                self.terminal_rows = snapshot.rows;
                Ok(LocalAttachmentEvent::Snapshot(snapshot))
            }
            WireKind::TerminalDelta => {
                let delta: v1::TerminalDelta = frame
                    .decode_message(WireKind::TerminalDelta)
                    .map_err(protocol_error)?;
                self.require_attachment(delta.attachment_id.clone())?;
                validate_product_viewport(delta.rows, delta.columns)?;
                validate_live_scroll_metrics(
                    delta.scroll_metrics.as_ref(),
                    delta.rows,
                    delta.to_revision,
                    delta.active_screen,
                )?;
                self.terminal_rows = delta.rows;
                Ok(LocalAttachmentEvent::Delta(delta))
            }
            WireKind::TerminalHistoryPage => {
                if self.pending_history_request_id != Some(frame.request_id) {
                    return Err(malformed("terminal history page correlation mismatch"));
                }
                let page: v1::TerminalHistoryPage = frame
                    .decode_message(WireKind::TerminalHistoryPage)
                    .map_err(protocol_error)?;
                self.require_attachment(page.attachment_id.clone())?;
                validate_history_page(&page)?;
                self.pending_history_request_id = None;
                Ok(LocalAttachmentEvent::HistoryPage(page))
            }
            WireKind::TerminalViewportFrame => {
                if self.pending_viewport_request_id != Some(frame.request_id) {
                    return Err(malformed("terminal viewport frame correlation mismatch"));
                }
                let viewport: v1::TerminalViewportFrame = frame
                    .decode_message(WireKind::TerminalViewportFrame)
                    .map_err(protocol_error)?;
                self.require_attachment(viewport.attachment_id.clone())?;
                validate_viewport_frame(&viewport, self.terminal_rows)?;
                self.pending_viewport_request_id = None;
                Ok(LocalAttachmentEvent::ViewportFrame(viewport))
            }
            WireKind::TerminalHistoryWindowFrame => {
                let Some((request_id, query)) = self.pending_history_window else {
                    return Err(malformed(
                        "terminal history window frame correlation mismatch",
                    ));
                };
                if request_id != frame.request_id {
                    return Err(malformed(
                        "terminal history window frame correlation mismatch",
                    ));
                }
                let window: v1::TerminalHistoryWindowFrame = frame
                    .decode_message(WireKind::TerminalHistoryWindowFrame)
                    .map_err(protocol_error)?;
                self.require_attachment(window.attachment_id.clone())?;
                validate_history_window_frame(&window, query)?;
                self.pending_history_window = None;
                Ok(LocalAttachmentEvent::HistoryWindowFrame(window))
            }
            WireKind::TerminalSyncRequired => {
                let required: v1::TerminalSyncRequired = frame
                    .decode_message(WireKind::TerminalSyncRequired)
                    .map_err(protocol_error)?;
                self.require_attachment(required.attachment_id.clone())?;
                Ok(LocalAttachmentEvent::SyncRequired(required))
            }
            WireKind::SessionMutateResponse => {
                if self.pending_takeover_request_id != Some(frame.request_id) {
                    return Err(malformed("takeover response correlation mismatch"));
                }
                self.pending_takeover_request_id = None;
                Ok(LocalAttachmentEvent::Takeover(mutate_response(frame)?))
            }
            WireKind::TerminalLeaseLost => {
                let lost: v1::TerminalLeaseLost = frame
                    .decode_message(WireKind::TerminalLeaseLost)
                    .map_err(protocol_error)?;
                self.require_attachment(lost.attachment_id.clone())?;
                Ok(LocalAttachmentEvent::LeaseLost(lost))
            }
            WireKind::TerminalSessionEnded => {
                let ended: v1::TerminalSessionEnded = frame
                    .decode_message(WireKind::TerminalSessionEnded)
                    .map_err(protocol_error)?;
                self.require_attachment(ended.attachment_id.clone())?;
                let session_id: SessionId = ended
                    .session_id
                    .clone()
                    .ok_or_else(|| malformed("session-ended event omitted session_id"))?
                    .try_into()
                    .map_err(protocol_error)?;
                if session_id != self.session_id {
                    return Err(malformed("session-ended event session_id mismatch"));
                }
                Ok(LocalAttachmentEvent::SessionEnded(ended))
            }
            kind => Err(malformed(format!(
                "wire kind {kind:?} is invalid from a terminal attachment"
            ))),
        }
    }

    /// Detaches this view while leaving the session and PTY running.
    pub async fn detach(&mut self) -> Result<(), DaemonError> {
        self.send(
            WireKind::TerminalDetach,
            &v1::TerminalDetach {
                attachment_id: Some(self.attachment_id.into()),
            },
        )
        .await?;
        self.stream
            .shutdown()
            .await
            .map_err(|error| local_attachment_io("finish local terminal detach", error))
    }

    async fn send<Message: prost::Message>(
        &mut self,
        kind: WireKind,
        message: &Message,
    ) -> Result<u64, DaemonError> {
        let request_id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or_else(|| resource_error("local attachment request ID exhausted"))?;
        let bytes = encode_message(kind, request_id, 0, message).map_err(protocol_error)?;
        self.stream
            .write_all(&bytes)
            .await
            .map_err(local_attachment_command_error)?;
        Ok(request_id)
    }

    async fn read_frame(&mut self) -> Result<DecodedFrame, DaemonError> {
        if let Some(frame) = self.deferred.pop_front() {
            return Ok(frame);
        }
        self.read_transport_frame().await
    }

    async fn read_transport_frame(&mut self) -> Result<DecodedFrame, DaemonError> {
        if let Some(frame) = self.queued.pop_front() {
            return Ok(frame);
        }
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = self
                .stream
                .read(&mut buffer)
                .await
                .map_err(|error| local_attachment_io("read local terminal event", error))?;
            if read == 0 {
                std::mem::replace(&mut self.decoder, FrameDecoder::new())
                    .finish()
                    .map_err(protocol_error)?;
                return Err(attachment_cancelled());
            }
            self.queued
                .extend(self.decoder.feed(&buffer[..read]).map_err(protocol_error)?);
            if let Some(frame) = self.queued.pop_front() {
                return Ok(frame);
            }
        }
    }

    async fn next_operation_id(&mut self) -> Result<OperationId, DaemonError> {
        if self.operation_lease.is_none() {
            let request_id = self.next_request_id;
            self.send(
                WireKind::SessionOperationLeaseRequest,
                &v1::SessionOperationLeaseRequest {
                    target: Some(resolved_target_wire(self.target)),
                },
            )
            .await?;
            loop {
                let frame = self.read_transport_frame().await?;
                if frame.request_id != request_id {
                    self.deferred.push_back(frame);
                    continue;
                }
                if frame.kind == WireKind::ServiceErrorResponse {
                    return Err(service_error(&frame)?);
                }
                if frame.kind != WireKind::SessionOperationLeaseResponse {
                    return Err(malformed("operation lease response kind mismatch"));
                }
                let response: v1::SessionOperationLeaseResponse = decode_response(&frame)?;
                self.operation_lease = Some(
                    response
                        .lease
                        .ok_or_else(|| malformed("operation lease response omitted lease"))?
                        .try_into()
                        .map_err(protocol_error)?,
                );
                break;
            }
        }
        let sequence = self.next_operation_sequence;
        self.next_operation_sequence = sequence.checked_add(1).ok_or_else(|| {
            self.operation_lease = None;
            self.next_operation_sequence = 1;
            resource_error("local attachment operation sequence exhausted")
        })?;
        Ok(OperationId {
            lease: self.operation_lease.expect("lease was allocated above"),
            sequence,
        })
    }

    fn require_snapshot_identity(
        &self,
        snapshot: &v1::TerminalSnapshot,
    ) -> Result<(), DaemonError> {
        if required_snapshot_session_id(snapshot)? != self.session_id {
            return Err(malformed("terminal snapshot session_id mismatch"));
        }
        self.require_attachment(snapshot.attachment_id.clone())
    }

    fn require_attachment(
        &self,
        attachment_id: Option<v1::AttachmentId>,
    ) -> Result<(), DaemonError> {
        let attachment_id: AttachmentId = attachment_id
            .ok_or_else(|| malformed("terminal event omitted attachment_id"))?
            .try_into()
            .map_err(protocol_error)?;
        if attachment_id == self.attachment_id {
            Ok(())
        } else {
            Err(malformed("terminal event attachment_id mismatch"))
        }
    }
}

#[cfg(unix)]
fn attachment_cancelled() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::Cancelled,
        "local terminal attachment closed",
    )
}

#[cfg(unix)]
fn local_attachment_command_error(error: std::io::Error) -> DaemonError {
    if is_attachment_closure_error(error.kind()) {
        DaemonError::new(DomainErrorKind::Cancelled, ATTACHMENT_COMMAND_STREAM_CLOSED)
    } else {
        daemon_io("write local terminal message", error)
    }
}

#[cfg(unix)]
fn local_attachment_io(operation: &str, error: std::io::Error) -> DaemonError {
    if is_attachment_closure_error(error.kind()) {
        attachment_cancelled()
    } else {
        daemon_io(operation, error)
    }
}

#[cfg(unix)]
const fn is_attachment_closure_error(kind: std::io::ErrorKind) -> bool {
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
pub(crate) fn is_attachment_command_stream_closed(error: &DaemonError) -> bool {
    error.kind() == DomainErrorKind::Cancelled && error.detail() == ATTACHMENT_COMMAND_STREAM_CLOSED
}

#[cfg(unix)]
pub(crate) fn is_attachment_stream_closed_without_event(error: &DaemonError) -> bool {
    error.kind() == DomainErrorKind::Cancelled
        && error.detail() == "local terminal attachment closed"
}

#[cfg(unix)]
fn service_error(frame: &DecodedFrame) -> Result<DaemonError, DaemonError> {
    let service_error: v1::ServiceError = frame
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
async fn read_frame_parts(
    stream: &mut tokio::net::UnixStream,
    decoder: &mut FrameDecoder,
    queued: &mut VecDeque<DecodedFrame>,
) -> Result<DecodedFrame, DaemonError> {
    if let Some(frame) = queued.pop_front() {
        return Ok(frame);
    }
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|error| daemon_io("read local terminal event", error))?;
        if read == 0 {
            std::mem::replace(decoder, FrameDecoder::new())
                .finish()
                .map_err(protocol_error)?;
            return Err(attachment_cancelled());
        }
        queued.extend(decoder.feed(&buffer[..read]).map_err(protocol_error)?);
        if let Some(frame) = queued.pop_front() {
            return Ok(frame);
        }
    }
}

#[cfg(unix)]
fn required_snapshot_session_id(snapshot: &v1::TerminalSnapshot) -> Result<SessionId, DaemonError> {
    snapshot
        .session_id
        .clone()
        .ok_or_else(|| malformed("terminal snapshot omitted session_id"))?
        .try_into()
        .map_err(protocol_error)
}

#[cfg(unix)]
fn required_snapshot_attachment_id(
    snapshot: &v1::TerminalSnapshot,
) -> Result<AttachmentId, DaemonError> {
    snapshot
        .attachment_id
        .clone()
        .ok_or_else(|| malformed("terminal snapshot omitted attachment_id"))?
        .try_into()
        .map_err(protocol_error)
}

#[cfg(unix)]
fn validate_snapshot_viewport(snapshot: &v1::TerminalSnapshot) -> Result<(), DaemonError> {
    validate_product_viewport(snapshot.rows, snapshot.columns)?;
    validate_live_scroll_metrics(
        snapshot.scroll_metrics.as_ref(),
        snapshot.rows,
        snapshot.revision,
        snapshot.active_screen,
    )?;
    Ok(())
}

#[cfg(unix)]
fn validate_product_viewport(rows: u32, columns: u32) -> Result<(), DaemonError> {
    let size: zterm_core::terminal::TerminalSize = v1::TerminalViewport { rows, columns }
        .try_into()
        .map_err(protocol_error)?;
    let limits = ResourceLimits::default();
    if size.rows > limits.max_viewport_rows || size.columns > limits.max_viewport_columns {
        return Err(malformed("terminal viewport exceeds product limits"));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_scroll_metrics(
    metrics: Option<&v1::TerminalScrollMetrics>,
    expected_rows: u32,
) -> Result<(), DaemonError> {
    if let Some(metrics) = metrics
        && (metrics.viewport_rows == 0
            || metrics.viewport_rows != expected_rows
            || metrics.viewport_rows > u32::from(ResourceLimits::default().max_viewport_rows)
            || metrics.epoch > metrics.revision
            || metrics.offset_from_bottom > metrics.max_offset_from_bottom)
    {
        return Err(malformed("terminal scroll metrics are inconsistent"));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_live_scroll_metrics(
    metrics: Option<&v1::TerminalScrollMetrics>,
    expected_rows: u32,
    expected_revision: u64,
    active_screen: i32,
) -> Result<(), DaemonError> {
    validate_scroll_metrics(metrics, expected_rows)?;
    let active_screen = v1::TerminalActiveScreen::try_from(active_screen)
        .map_err(|_| malformed("terminal update used an unknown active screen"))?;
    if matches!(active_screen, v1::TerminalActiveScreen::Unspecified) {
        return Err(malformed(
            "terminal update used an unspecified active screen",
        ));
    }
    if let Some(metrics) = metrics
        && (metrics.offset_from_bottom != 0
            || metrics.revision != expected_revision
            || metrics.epoch > metrics.revision
            || active_screen != v1::TerminalActiveScreen::Main)
    {
        return Err(malformed("terminal live scroll metrics are inconsistent"));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_viewport_frame(
    frame: &v1::TerminalViewportFrame,
    expected_rows: u32,
) -> Result<(), DaemonError> {
    let outcome = v1::TerminalViewportOutcome::try_from(frame.outcome)
        .map_err(|_| malformed("terminal viewport outcome is invalid"))?;
    match outcome {
        v1::TerminalViewportOutcome::Frame => {
            let metrics = frame
                .metrics
                .as_ref()
                .ok_or_else(|| malformed("terminal viewport frame omitted metrics"))?;
            validate_scroll_metrics(Some(metrics), expected_rows)?;
            let disposition = v1::TerminalViewportDisposition::try_from(frame.disposition)
                .map_err(|_| malformed("terminal viewport disposition is invalid"))?;
            if matches!(disposition, v1::TerminalViewportDisposition::Unspecified)
                || metrics.offset_from_bottom == 0
                || usize::try_from(metrics.viewport_rows).ok() != Some(frame.rows.len())
                || metrics.epoch != frame.current_epoch
                || metrics.revision != frame.current_revision
            {
                return Err(malformed("terminal viewport frame is inconsistent"));
            }
        }
        v1::TerminalViewportOutcome::Live => {
            let metrics = frame
                .metrics
                .as_ref()
                .ok_or_else(|| malformed("terminal live viewport omitted metrics"))?;
            validate_scroll_metrics(Some(metrics), expected_rows)?;
            if metrics.offset_from_bottom != 0
                || !frame.rows.is_empty()
                || frame.disposition != v1::TerminalViewportDisposition::Unspecified as i32
                || metrics.epoch != frame.current_epoch
                || metrics.revision != frame.current_revision
            {
                return Err(malformed("terminal live viewport is inconsistent"));
            }
        }
        v1::TerminalViewportOutcome::Changed | v1::TerminalViewportOutcome::Gap => {
            if frame.metrics.is_some()
                || !frame.rows.is_empty()
                || frame.disposition != v1::TerminalViewportDisposition::Unspecified as i32
                || frame.current_epoch > frame.current_revision
            {
                return Err(malformed("terminal viewport reset retained frame content"));
            }
        }
        v1::TerminalViewportOutcome::Unspecified => {
            return Err(malformed("terminal viewport outcome is invalid"));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn validate_history_window_frame(
    frame: &v1::TerminalHistoryWindowFrame,
    query: TerminalHistoryWindowQuery,
) -> Result<(), DaemonError> {
    let outcome = v1::TerminalHistoryWindowOutcome::try_from(frame.outcome)
        .map_err(|_| malformed("terminal history window outcome is invalid"))?;
    match outcome {
        v1::TerminalHistoryWindowOutcome::Frame => {
            let anchor = frame
                .anchor
                .as_ref()
                .ok_or_else(|| malformed("terminal history window frame omitted anchor"))?;
            validate_product_viewport(anchor.viewport_rows, anchor.viewport_columns)?;
            let disposition = match v1::TerminalViewportDisposition::try_from(frame.disposition)
                .map_err(|_| malformed("terminal history window disposition is invalid"))?
            {
                v1::TerminalViewportDisposition::Exact => TerminalViewportDisposition::Exact,
                v1::TerminalViewportDisposition::Rebased => TerminalViewportDisposition::Rebased,
                v1::TerminalViewportDisposition::Unspecified => {
                    return Err(malformed("terminal history window disposition is invalid"));
                }
            };
            let response_anchor = TerminalHistoryWindowAnchor {
                epoch: Revision::new(anchor.epoch),
                revision: Revision::new(anchor.revision),
                max_offset_from_bottom: anchor.max_offset_from_bottom,
                viewport: TerminalSize::new(
                    u16::try_from(anchor.viewport_rows).map_err(|_| {
                        malformed("terminal history window rows are not representable")
                    })?,
                    u16::try_from(anchor.viewport_columns).map_err(|_| {
                        malformed("terminal history window columns are not representable")
                    })?,
                ),
            };
            let shape = query.response_shape(response_anchor).ok_or_else(|| {
                malformed("terminal history window predates or contradicts its request")
            })?;
            if anchor.epoch > anchor.revision
                || anchor.epoch != frame.current_epoch
                || anchor.revision != frame.current_revision
                || disposition != shape.disposition
                || frame.target_offset_from_bottom != shape.target_offset_from_bottom
                || frame.first_row_from_live_top != shape.first_row_from_live_top
                || frame.ansi_rows.len() != shape.row_count
            {
                return Err(malformed("terminal history window frame is inconsistent"));
            }
        }
        v1::TerminalHistoryWindowOutcome::Changed | v1::TerminalHistoryWindowOutcome::Gap => {
            if frame.anchor.is_some()
                || !frame.ansi_rows.is_empty()
                || frame.disposition != v1::TerminalViewportDisposition::Unspecified as i32
                || frame.target_offset_from_bottom != 0
                || frame.first_row_from_live_top != 0
                || frame.current_epoch > frame.current_revision
                || ((frame.current_epoch, frame.current_revision) != (0, 0)
                    && frame.current_revision < query.anchor.revision.get())
            {
                return Err(malformed(
                    "terminal history window reset retained frame content",
                ));
            }
        }
        v1::TerminalHistoryWindowOutcome::Unspecified => {
            return Err(malformed("terminal history window outcome is invalid"));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn validate_history_page(page: &v1::TerminalHistoryPage) -> Result<(), DaemonError> {
    let outcome = v1::TerminalHistoryOutcome::try_from(page.outcome)
        .map_err(|_| malformed("terminal history outcome is invalid"))?;
    if page.rows.len() > MAX_HISTORY_PAGE_ROWS {
        return Err(malformed("terminal history page exceeded the row bound"));
    }
    match outcome {
        v1::TerminalHistoryOutcome::Ok => {
            let cursor = page
                .cursor
                .as_ref()
                .ok_or_else(|| malformed("terminal history page omitted its cursor"))?;
            if usize::try_from(cursor.row_count).ok() != Some(page.rows.len())
                || cursor.epoch != page.current_epoch
                || cursor.revision != page.current_revision
            {
                return Err(malformed("terminal history page cursor is inconsistent"));
            }
        }
        v1::TerminalHistoryOutcome::Changed | v1::TerminalHistoryOutcome::Gap => {
            if page.cursor.is_some() || !page.rows.is_empty() {
                return Err(malformed(
                    "terminal history reset outcome retained page content",
                ));
            }
        }
        v1::TerminalHistoryOutcome::Unspecified => {
            return Err(malformed("terminal history outcome is invalid"));
        }
    }
    Ok(())
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
    let mut buffer = Zeroizing::new([0_u8; 16 * 1024]);
    let mut completed = None;
    loop {
        let read = stream
            .read(&mut *buffer)
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
pub struct LocalClient {
    socket: PathBuf,
    #[cfg(unix)]
    next_request_id: AtomicU64,
    #[cfg(unix)]
    mutation_targets:
        StdMutex<BTreeMap<ResolvedSessionTarget, Arc<AsyncMutex<LocalMutationState>>>>,
}

impl fmt::Debug for LocalClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("LocalClient");
        debug.field("socket", &"[REDACTED]");
        #[cfg(unix)]
        {
            let mutation_target_count = self
                .mutation_targets
                .try_lock()
                .ok()
                .map(|targets| targets.len());
            debug
                .field(
                    "next_request_id",
                    &self.next_request_id.load(Ordering::Relaxed),
                )
                .field("mutation_target_count", &mutation_target_count);
        }
        debug.finish_non_exhaustive()
    }
}

#[cfg(unix)]
struct LocalMutationState {
    lease: Option<OperationLease>,
    next_sequence: u64,
}

#[cfg(unix)]
impl fmt::Debug for LocalMutationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalMutationState")
            .field("has_lease", &self.lease.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(unix)]
enum LocalRemoteAttemptError {
    PreWrite(DaemonError),
    PostWrite(DaemonError),
    Complete(DaemonError),
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalRemoteRequestClass {
    ReadOnly,
    StatefulControl,
    Mutation,
}

#[cfg(unix)]
impl LocalRemoteRequestClass {
    fn for_kind(kind: WireKind) -> Result<Self, DaemonError> {
        match kind {
            WireKind::SessionListRequest => Ok(Self::ReadOnly),
            WireKind::SessionOperationLeaseRequest => Ok(Self::StatefulControl),
            WireKind::SessionCreateRequest
            | WireKind::SessionRenameRequest
            | WireKind::SessionCloseRequest
            | WireKind::SessionTakeoverRequest => Ok(Self::Mutation),
            _ => Err(malformed(
                "local remote-Session envelope contains a non-unary Session kind",
            )),
        }
    }
}

impl LocalClient {
    /// Creates a non-spawning client for one effective user's daemon socket.
    #[must_use]
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        #[cfg(unix)]
        let mutation_targets = BTreeMap::from([(
            ResolvedSessionTarget::local(),
            Arc::new(AsyncMutex::new(LocalMutationState {
                lease: None,
                next_sequence: 1,
            })),
        )]);
        Self {
            socket: socket.into(),
            #[cfg(unix)]
            next_request_id: AtomicU64::new(1),
            #[cfg(unix)]
            mutation_targets: StdMutex::new(mutation_targets),
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
            .clone()
            .ok_or_else(|| malformed("status response omitted device_id"))?
            .try_into()
            .map_err(protocol_error)?;
        let network = network_observation(&response, device_id)?;
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
            network,
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

    /// Resolves one exact user selector inside the daemon and returns a frozen
    /// target token containing no alias.
    #[cfg(unix)]
    pub async fn resolve_session_target(
        &self,
        selector: &str,
    ) -> Result<ResolvedSessionTarget, DaemonError> {
        let frame = self
            .request(
                WireKind::LocalTargetResolveRequest,
                WireKind::LocalTargetResolveResponse,
                &v1::LocalTargetResolveRequest {
                    selector: selector.to_owned(),
                },
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v1::LocalTargetResolveResponse = decode_response(&frame)?;
        resolved_target_from_wire(response.target)
    }

    /// Lists live sessions on the local daemon through one strict unary request.
    #[cfg(unix)]
    pub async fn list_sessions(&self) -> Result<Vec<crate::session::SessionSummary>, DaemonError> {
        self.list_sessions_at(ResolvedSessionTarget::local()).await
    }

    /// Lists live sessions on one already-resolved exact target.
    #[cfg(unix)]
    pub async fn list_sessions_at(
        &self,
        target: ResolvedSessionTarget,
    ) -> Result<Vec<crate::session::SessionSummary>, DaemonError> {
        let frame = self
            .session_request(
                target,
                WireKind::SessionListRequest,
                WireKind::SessionListResponse,
                &v1::SessionListRequest {
                    target: Some(resolved_target_wire(target)),
                },
                DEFAULT_DEADLINE,
                false,
            )
            .await?;
        let response: v1::SessionListResponse = decode_response(&frame)?;
        response
            .sessions
            .into_iter()
            .map(session_summary_from_wire)
            .collect()
    }

    /// Creates a named account-login-shell session.
    #[cfg(unix)]
    pub async fn create_session(
        &self,
        name: &SessionName,
        working_directory: Option<&Path>,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        self.create_session_at(
            ResolvedSessionTarget::local(),
            name,
            working_directory,
            viewport,
        )
        .await
    }

    /// Creates a named account-login-shell session on one exact target.
    #[cfg(unix)]
    pub async fn create_session_at(
        &self,
        target: ResolvedSessionTarget,
        name: &SessionName,
        working_directory: Option<&Path>,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        let frame = self
            .mutation_request(target, WireKind::SessionCreateRequest, |operation_id| {
                v1::SessionCreateRequest {
                    operation_id: Some(operation_id.into()),
                    target: Some(resolved_target_wire(target)),
                    name: name.to_string(),
                    working_directory: working_directory
                        .map_or_else(String::new, |path| path.to_string_lossy().into_owned()),
                    viewport: viewport.map(Into::into),
                }
            })
            .await?;
        mutate_response(frame)
    }

    /// Renames a live session without changing its identity.
    #[cfg(unix)]
    pub async fn rename_session(
        &self,
        session_id: SessionId,
        name: &SessionName,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        self.rename_session_at(ResolvedSessionTarget::local(), session_id, name)
            .await
    }

    /// Renames a live session on one exact target without changing its identity.
    #[cfg(unix)]
    pub async fn rename_session_at(
        &self,
        target: ResolvedSessionTarget,
        session_id: SessionId,
        name: &SessionName,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        let frame = self
            .mutation_request(target, WireKind::SessionRenameRequest, |operation_id| {
                v1::SessionRenameRequest {
                    operation_id: Some(operation_id.into()),
                    target: Some(resolved_target_wire(target)),
                    session_id: Some(session_id.into()),
                    name: name.to_string(),
                }
            })
            .await?;
        mutate_response(frame)
    }

    /// Explicitly closes one live session.
    #[cfg(unix)]
    pub async fn close_session(
        &self,
        session_id: SessionId,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        self.close_session_at(ResolvedSessionTarget::local(), session_id)
            .await
    }

    /// Explicitly closes one live session on an exact target.
    #[cfg(unix)]
    pub async fn close_session_at(
        &self,
        target: ResolvedSessionTarget,
        session_id: SessionId,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        let frame = self
            .mutation_request(target, WireKind::SessionCloseRequest, |operation_id| {
                v1::SessionCloseRequest {
                    operation_id: Some(operation_id.into()),
                    target: Some(resolved_target_wire(target)),
                    session_id: Some(session_id.into()),
                }
            })
            .await?;
        mutate_response(frame)
    }

    #[cfg(unix)]
    async fn mutation_request<Message, Build>(
        &self,
        target: ResolvedSessionTarget,
        request_kind: WireKind,
        build: Build,
    ) -> Result<DecodedFrame, DaemonError>
    where
        Message: prost::Message,
        Build: FnOnce(OperationId) -> Message,
    {
        // Only one exact target is serialized. No remote await holds the map
        // mutex or blocks local/other-device lease streams.
        let state = self.mutation_target_state(target)?;
        let mut mutation = state.lock().await;
        if mutation.lease.is_none() {
            mutation.lease = Some(self.issue_operation_lease(target).await?);
            mutation.next_sequence = 1;
        }
        let sequence = mutation.next_sequence;
        mutation.next_sequence = match sequence.checked_add(1) {
            Some(next) => next,
            None => {
                mutation.lease = None;
                mutation.next_sequence = 1;
                return Err(resource_error("local operation sequence exhausted"));
            }
        };
        let operation_id = OperationId {
            lease: mutation.lease.expect("lease was allocated above"),
            sequence,
        };
        let result = self
            .session_request(
                target,
                request_kind,
                WireKind::SessionMutateResponse,
                &build(operation_id),
                DEFAULT_DEADLINE,
                true,
            )
            .await;
        if result
            .as_ref()
            .err()
            .is_some_and(|error| error.kind() == DomainErrorKind::OperationOutcomeUnknown)
        {
            mutation.lease = None;
            mutation.next_sequence = 1;
        }
        result
    }

    #[cfg(unix)]
    async fn issue_operation_lease(
        &self,
        target: ResolvedSessionTarget,
    ) -> Result<OperationLease, DaemonError> {
        let frame = self
            .session_request(
                target,
                WireKind::SessionOperationLeaseRequest,
                WireKind::SessionOperationLeaseResponse,
                &v1::SessionOperationLeaseRequest {
                    target: Some(resolved_target_wire(target)),
                },
                DEFAULT_DEADLINE,
                true,
            )
            .await?;
        let response: v1::SessionOperationLeaseResponse = decode_response(&frame)?;
        response
            .lease
            .ok_or_else(|| malformed("operation lease response omitted lease"))?
            .try_into()
            .map_err(protocol_error)
    }

    #[cfg(unix)]
    fn mutation_target_state(
        &self,
        target: ResolvedSessionTarget,
    ) -> Result<Arc<AsyncMutex<LocalMutationState>>, DaemonError> {
        let mut states = self
            .mutation_targets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(state) = states.get(&target) {
            return Ok(Arc::clone(state));
        }
        if states.len() >= MAX_MUTATION_TARGETS_PER_CLIENT {
            // The map is the only source of new Arcs while this mutex is held.
            // A strong count of one therefore proves that no logical mutation
            // or waiter can still use this target state; cached inactive leases
            // may be discarded, but in-flight operation identity is never evicted.
            let inactive = states
                .iter()
                .find_map(|(target, state)| (Arc::strong_count(state) == 1).then_some(*target));
            let Some(inactive) = inactive else {
                return Err(resource_error(
                    "local client mutation-target capacity is exhausted by active operations",
                ));
            };
            states.remove(&inactive);
        }
        let state = Arc::new(AsyncMutex::new(LocalMutationState {
            lease: None,
            next_sequence: 1,
        }));
        states.insert(target, Arc::clone(&state));
        Ok(state)
    }

    #[cfg(unix)]
    async fn session_request<Message: prost::Message>(
        &self,
        target: ResolvedSessionTarget,
        request_kind: WireKind,
        response_kind: WireKind,
        message: &Message,
        deadline: Duration,
        mutation_or_lease_retry: bool,
    ) -> Result<DecodedFrame, DaemonError> {
        let request_id = self
            .next_request_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| resource_error("local request ID exhausted"))?;
        let deadline_ms = u32::try_from(deadline.as_millis()).unwrap_or(u32::MAX);
        let bytes = Zeroizing::new(
            encode_message(request_kind, request_id, deadline_ms, message)
                .map_err(protocol_error)?,
        );
        match target.device_id() {
            None => {
                self.request_preencoded(
                    &bytes,
                    request_id,
                    response_kind,
                    deadline,
                    mutation_or_lease_retry,
                )
                .await
            }
            Some(device_id) => {
                let request_class = LocalRemoteRequestClass::for_kind(request_kind)?;
                self.request_remote_preencoded(
                    device_id,
                    &bytes,
                    request_id,
                    response_kind,
                    deadline,
                    request_class,
                )
                .await
            }
        }
    }

    #[cfg(unix)]
    async fn request_remote_preencoded(
        &self,
        target: DeviceId,
        bytes: &[u8],
        request_id: u64,
        response_kind: WireKind,
        deadline: Duration,
        request_class: LocalRemoteRequestClass,
    ) -> Result<DecodedFrame, DaemonError> {
        let mut envelope = v1::LocalSessionUnaryRequest {
            target_device_id: Some(target.into()),
            frame: bytes.to_vec(),
        };
        let deadline_ms = u32::try_from(deadline.as_millis()).unwrap_or(u32::MAX);
        let outer = Zeroizing::new(
            encode_message(
                WireKind::LocalSessionUnaryRequest,
                request_id,
                deadline_ms,
                &envelope,
            )
            .map_err(protocol_error)?,
        );
        envelope.frame.zeroize();
        let absolute_deadline = Instant::now() + deadline;
        let first = self
            .request_remote_attempt(&outer, request_id, response_kind, absolute_deadline)
            .await;
        match first {
            Ok(frame) => Ok(frame),
            Err(
                LocalRemoteAttemptError::PreWrite(error) | LocalRemoteAttemptError::Complete(error),
            ) => Err(error),
            Err(LocalRemoteAttemptError::PostWrite(first_error)) => match request_class {
                LocalRemoteRequestClass::Mutation => Err(DaemonError::new(
                    DomainErrorKind::OperationOutcomeUnknown,
                    "remote Session mutation may have committed but no complete local reply was received",
                )),
                LocalRemoteRequestClass::StatefulControl => Err(first_error),
                LocalRemoteRequestClass::ReadOnly => match self
                    .request_remote_attempt(&outer, request_id, response_kind, absolute_deadline)
                    .await
                {
                    Ok(frame) => Ok(frame),
                    Err(LocalRemoteAttemptError::Complete(error)) => Err(error),
                    Err(
                        LocalRemoteAttemptError::PreWrite(error)
                        | LocalRemoteAttemptError::PostWrite(error),
                    ) => Err(error),
                },
            },
        }
    }

    #[cfg(unix)]
    async fn request_remote_attempt(
        &self,
        bytes: &[u8],
        request_id: u64,
        response_kind: WireKind,
        absolute_deadline: Instant,
    ) -> Result<DecodedFrame, LocalRemoteAttemptError> {
        let frame = self
            .request_remote_bytes_once(bytes, absolute_deadline)
            .await?;
        match validate_session_unary_response(&frame, request_id, response_kind)
            .map_err(LocalRemoteAttemptError::PostWrite)?
        {
            SessionUnaryResponseStatus::Expected => Ok(frame),
            SessionUnaryResponseStatus::ServiceError(error) => {
                Err(LocalRemoteAttemptError::Complete(error))
            }
        }
    }

    #[cfg(unix)]
    async fn request_remote_bytes_once(
        &self,
        bytes: &[u8],
        absolute_deadline: Instant,
    ) -> Result<DecodedFrame, LocalRemoteAttemptError> {
        let remaining = absolute_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(LocalRemoteAttemptError::PreWrite(DaemonError::new(
                DomainErrorKind::DeadlineExceeded,
                "local forwarding deadline elapsed before connect",
            )));
        }
        let mut stream =
            tokio::time::timeout(remaining, tokio::net::UnixStream::connect(&self.socket))
                .await
                .map_err(|_| {
                    LocalRemoteAttemptError::PreWrite(DaemonError::new(
                        DomainErrorKind::DeadlineExceeded,
                        "local forwarding deadline elapsed before connect",
                    ))
                })?
                .map_err(|error| LocalRemoteAttemptError::PreWrite(connect_error(error)))?;

        let mut written = 0;
        while written < bytes.len() {
            let remaining = absolute_deadline.saturating_duration_since(Instant::now());
            let write = tokio::time::timeout(remaining, stream.write(&bytes[written..]))
                .await
                .map_err(|_| {
                    let error = DaemonError::new(
                        DomainErrorKind::DeadlineExceeded,
                        "local forwarding request write exceeded its deadline",
                    );
                    if written == 0 {
                        LocalRemoteAttemptError::PreWrite(error)
                    } else {
                        LocalRemoteAttemptError::PostWrite(error)
                    }
                })?
                .map_err(|error| {
                    let error = daemon_io("write local forwarding request", error);
                    if written == 0 {
                        LocalRemoteAttemptError::PreWrite(error)
                    } else {
                        LocalRemoteAttemptError::PostWrite(error)
                    }
                })?;
            if write == 0 {
                let error = daemon_io(
                    "write local forwarding request",
                    std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "local socket accepted zero request bytes",
                    ),
                );
                return Err(if written == 0 {
                    LocalRemoteAttemptError::PreWrite(error)
                } else {
                    LocalRemoteAttemptError::PostWrite(error)
                });
            }
            written += write;
        }

        let remaining = absolute_deadline.saturating_duration_since(Instant::now());
        tokio::time::timeout(remaining, stream.shutdown())
            .await
            .map_err(|_| {
                LocalRemoteAttemptError::PostWrite(DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    "local forwarding request finish exceeded its deadline",
                ))
            })?
            .map_err(|error| {
                LocalRemoteAttemptError::PostWrite(daemon_io(
                    "finish local forwarding request",
                    error,
                ))
            })?;

        let remaining = absolute_deadline.saturating_duration_since(Instant::now());
        tokio::time::timeout(remaining, read_one(&mut stream))
            .await
            .map_err(|_| {
                LocalRemoteAttemptError::PostWrite(DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    "local forwarding response exceeded its deadline",
                ))
            })?
            .map_err(LocalRemoteAttemptError::PostWrite)
    }

    #[cfg(unix)]
    async fn request<Message: prost::Message>(
        &self,
        request_kind: WireKind,
        response_kind: WireKind,
        message: &Message,
        deadline: Duration,
    ) -> Result<DecodedFrame, DaemonError> {
        self.request_encoded(request_kind, response_kind, message, deadline, false)
            .await
    }

    #[cfg(unix)]
    async fn request_with_retry<Message: prost::Message>(
        &self,
        request_kind: WireKind,
        response_kind: WireKind,
        message: &Message,
        deadline: Duration,
    ) -> Result<DecodedFrame, DaemonError> {
        self.request_encoded(request_kind, response_kind, message, deadline, true)
            .await
    }

    #[cfg(unix)]
    async fn request_encoded<Message: prost::Message>(
        &self,
        request_kind: WireKind,
        response_kind: WireKind,
        message: &Message,
        deadline: Duration,
        retry_ambiguous: bool,
    ) -> Result<DecodedFrame, DaemonError> {
        let request_id = self
            .next_request_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| resource_error("local request ID exhausted"))?;
        let deadline_ms = u32::try_from(deadline.as_millis()).unwrap_or(u32::MAX);
        let bytes = Zeroizing::new(
            encode_message(request_kind, request_id, deadline_ms, message)
                .map_err(protocol_error)?,
        );
        self.request_preencoded(&bytes, request_id, response_kind, deadline, retry_ambiguous)
            .await
    }

    #[cfg(unix)]
    async fn request_pair_accept(
        &self,
        mut message: v1::LocalPairAcceptRequest,
    ) -> Result<DecodedFrame, DaemonError> {
        let request_id = self
            .next_request_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| resource_error("local request ID exhausted"))?;
        let deadline_ms = u32::try_from(PAIRING_DEADLINE.as_millis()).unwrap_or(u32::MAX);
        let bytes = Zeroizing::new(
            encode_message(
                WireKind::LocalPairAcceptRequest,
                request_id,
                deadline_ms,
                &message,
            )
            .map_err(protocol_error)?,
        );
        message.ticket.zeroize();
        self.request_preencoded(
            &bytes,
            request_id,
            WireKind::LocalPairAcceptResponse,
            PAIRING_DEADLINE,
            true,
        )
        .await
    }

    #[cfg(unix)]
    async fn request_preencoded(
        &self,
        bytes: &[u8],
        request_id: u64,
        response_kind: WireKind,
        deadline: Duration,
        retry_ambiguous: bool,
    ) -> Result<DecodedFrame, DaemonError> {
        let absolute_deadline = Instant::now() + deadline;
        let attempts = if retry_ambiguous { 2 } else { 1 };
        let mut last_error = None;
        for _ in 0..attempts {
            match self.request_bytes_once(bytes, absolute_deadline).await {
                Ok(frame) => {
                    // Any complete response is definitive, including a typed
                    // OutcomeUnknown. Only transport ambiguity may consume the
                    // single byte-identical retry.
                    if frame.request_id != request_id {
                        return Err(malformed("local response request_id mismatch"));
                    }
                    if frame.kind == WireKind::ServiceErrorResponse {
                        return Err(service_error(&frame)?);
                    }
                    if frame.kind != response_kind {
                        return Err(malformed(format!(
                            "expected {response_kind:?}, got {:?}",
                            frame.kind
                        )));
                    }
                    return Ok(frame);
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| resource_error("local request had no attempt")))
    }

    #[cfg(unix)]
    async fn request_bytes_once(
        &self,
        bytes: &[u8],
        absolute_deadline: Instant,
    ) -> Result<DecodedFrame, DaemonError> {
        let remaining = absolute_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(DaemonError::new(
                DomainErrorKind::DeadlineExceeded,
                "local request deadline elapsed",
            ));
        }
        tokio::time::timeout(remaining, self.request_bytes_once_inner(bytes))
            .await
            .map_err(|_| {
                DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    "timed out waiting for local daemon response",
                )
            })?
    }

    #[cfg(unix)]
    async fn request_bytes_once_inner(&self, bytes: &[u8]) -> Result<DecodedFrame, DaemonError> {
        let mut stream = tokio::net::UnixStream::connect(&self.socket)
            .await
            .map_err(connect_error)?;
        stream
            .write_all(bytes)
            .await
            .map_err(|error| daemon_io("write local request", error))?;
        stream
            .shutdown()
            .await
            .map_err(|error| daemon_io("finish local request", error))?;
        read_one(&mut stream).await
    }
}

/// Real same-UID unary device-management adapter used by daemon integration
/// tests and the high-level command runtime. It never opens SQLite, reads the
/// identity key, binds Iroh, or starts a daemon.
#[derive(Debug)]
#[doc(hidden)]
pub struct LocalDeviceClient {
    #[cfg(unix)]
    client: LocalClient,
}

impl LocalDeviceClient {
    /// Creates a non-spawning device client for one daemon socket.
    #[must_use]
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        #[cfg(unix)]
        {
            Self {
                client: LocalClient::new(socket),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = socket;
            Self {}
        }
    }

    /// Lists the directional outbound/inbound projection of every device.
    #[cfg(unix)]
    pub async fn list(&self) -> Result<Vec<DeviceSummary>, DaemonError> {
        let frame = self
            .client
            .request(
                WireKind::LocalDeviceListRequest,
                WireKind::LocalDeviceListResponse,
                &v1::LocalDeviceListRequest {},
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v1::LocalDeviceListResponse = decode_response(&frame)?;
        response
            .devices
            .into_iter()
            .map(|device| device.try_into().map_err(local_device_wire_error))
            .collect()
    }

    /// Sets the exact outbound alias for one exact DeviceId.
    #[cfg(unix)]
    pub async fn rename(
        &self,
        device_id: DeviceId,
        alias: &DeviceAlias,
    ) -> Result<DeviceSummary, DaemonError> {
        let frame = self
            .client
            .request_with_retry(
                WireKind::LocalDeviceRenameRequest,
                WireKind::LocalDeviceRenameResponse,
                &v1::LocalDeviceRenameRequest {
                    device_id: Some(device_id.into()),
                    alias: alias.as_str().to_owned(),
                },
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v1::LocalDeviceRenameResponse = decode_response(&frame)?;
        response
            .device
            .ok_or_else(|| malformed("device rename response omitted device"))?
            .try_into()
            .map_err(local_device_wire_error)
    }

    /// Revokes only the inbound authorization for one exact DeviceId.
    #[cfg(unix)]
    pub async fn revoke(&self, device_id: DeviceId) -> Result<DeviceSummary, DaemonError> {
        let frame = self
            .client
            .request_with_retry(
                WireKind::LocalDeviceRevokeRequest,
                WireKind::LocalDeviceRevokeResponse,
                &v1::LocalDeviceRevokeRequest {
                    device_id: Some(device_id.into()),
                },
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v1::LocalDeviceRevokeResponse = decode_response(&frame)?;
        response
            .device
            .ok_or_else(|| malformed("device revoke response omitted device"))?
            .try_into()
            .map_err(local_device_wire_error)
    }

    /// Returns the current platform limitation.
    #[cfg(not(unix))]
    pub async fn list(&self) -> Result<Vec<DeviceSummary>, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation.
    #[cfg(not(unix))]
    pub async fn rename(
        &self,
        _device_id: DeviceId,
        _alias: &DeviceAlias,
    ) -> Result<DeviceSummary, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation.
    #[cfg(not(unix))]
    pub async fn revoke(&self, _device_id: DeviceId) -> Result<DeviceSummary, DaemonError> {
        Err(unsupported())
    }
}

/// Hidden same-UID pairing adapter used by integration tests and the command
/// composition. It never starts a daemon or opens an Iroh endpoint itself.
#[cfg(unix)]
#[derive(Debug)]
#[doc(hidden)]
pub struct LocalPairingClient {
    client: LocalClient,
}

#[cfg(unix)]
impl LocalPairingClient {
    /// Creates a non-spawning pairing client for one daemon socket.
    #[must_use]
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            client: LocalClient::new(socket),
        }
    }

    /// Creates one replay-safe bearer ticket. A zero TTL selects the product
    /// default before the semantic fingerprint is computed.
    pub async fn create(&self, ttl_seconds: u32) -> Result<PairTicketText, DaemonError> {
        let effective_ttl = if ttl_seconds == 0 {
            DEFAULT_PAIR_TTL_SECONDS
        } else {
            u64::from(ttl_seconds)
        };
        let operation_id = random_pair_operation_id()?;
        let fingerprint = PairFingerprint::for_create(effective_ttl);
        let mut frame = self
            .client
            .request_with_retry(
                WireKind::LocalPairCreateRequest,
                WireKind::LocalPairCreateResponse,
                &v1::LocalPairCreateRequest {
                    ephemeral_operation_id: operation_id.as_bytes().to_vec(),
                    fingerprint: fingerprint.as_bytes().to_vec(),
                    ttl_seconds,
                },
                PAIRING_DEADLINE,
            )
            .await?;
        let response = decode_response::<v1::LocalPairCreateResponse>(&frame);
        frame.payload.zeroize();
        let response = response?;
        PairTicketText::from_local_response(response.ticket).map_err(DaemonError::from)
    }

    /// Accepts one bearer ticket in the outbound direction. The ticket and its
    /// encoded request are zeroized after the byte-identical retry window.
    pub async fn accept(
        &self,
        ticket: PairTicketText,
        alias: Option<&DeviceAlias>,
    ) -> Result<DeviceSummary, DaemonError> {
        let operation_id = random_pair_operation_id()?;
        let fingerprint = PairFingerprint::for_accept(ticket.expose().as_bytes(), alias);
        let request = v1::LocalPairAcceptRequest {
            ephemeral_operation_id: operation_id.as_bytes().to_vec(),
            fingerprint: fingerprint.as_bytes().to_vec(),
            ticket: ticket.expose().to_owned(),
            alias: alias.map_or_else(String::new, |alias| alias.as_str().to_owned()),
        };
        let result = self.client.request_pair_accept(request).await;
        drop(ticket);
        let mut frame = result?;
        let response = decode_response::<v1::LocalPairAcceptResponse>(&frame);
        frame.payload.zeroize();
        response?
            .device
            .ok_or_else(|| malformed("pair accept response omitted device"))?
            .try_into()
            .map_err(local_device_wire_error)
    }
}

#[cfg(unix)]
fn random_pair_operation_id() -> Result<EphemeralOperationId, DaemonError> {
    let mut bytes = [0_u8; EphemeralOperationId::LENGTH];
    SystemRandom::new().fill(&mut bytes).map_err(|_| {
        DaemonError::new(
            DomainErrorKind::TransportUnavailable,
            "operating-system randomness is unavailable for a pairing operation",
        )
    })?;
    Ok(EphemeralOperationId::from_array(bytes))
}

#[cfg(not(unix))]
impl LocalClient {
    /// Returns the current platform limitation on non-Unix targets.
    pub async fn readiness(&self) -> Result<DaemonReadiness, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation on non-Unix targets.
    pub async fn status(&self) -> Result<DaemonStatus, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation on non-Unix targets.
    pub async fn validate_setup(
        &self,
        _requested: &ValidatedConfig,
    ) -> Result<ValidatedSetupStatus, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation on non-Unix targets.
    pub async fn stop(&self, _force: bool) -> Result<SessionImpact, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation on non-Unix targets.
    pub async fn update_preflight(&self) -> Result<SessionImpact, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation.
    pub async fn resolve_session_target(
        &self,
        _selector: &str,
    ) -> Result<ResolvedSessionTarget, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation.
    pub async fn list_sessions(&self) -> Result<Vec<crate::session::SessionSummary>, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation.
    pub async fn list_sessions_at(
        &self,
        _target: ResolvedSessionTarget,
    ) -> Result<Vec<crate::session::SessionSummary>, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation.
    pub async fn create_session(
        &self,
        _name: &SessionName,
        _working_directory: Option<&Path>,
        _viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation.
    pub async fn create_session_at(
        &self,
        _target: ResolvedSessionTarget,
        _name: &SessionName,
        _working_directory: Option<&Path>,
        _viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation.
    pub async fn rename_session(
        &self,
        _session_id: SessionId,
        _name: &SessionName,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation.
    pub async fn rename_session_at(
        &self,
        _target: ResolvedSessionTarget,
        _session_id: SessionId,
        _name: &SessionName,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation.
    pub async fn close_session(
        &self,
        _session_id: SessionId,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        Err(unsupported())
    }

    /// Returns the current platform limitation.
    pub async fn close_session_at(
        &self,
        _target: ResolvedSessionTarget,
        _session_id: SessionId,
    ) -> Result<crate::session::SessionSummary, DaemonError> {
        Err(unsupported())
    }
}

#[cfg(unix)]
fn local_device_wire_error(error: zterm_proto::WireFieldError) -> DaemonError {
    malformed(format!("invalid local device response: {error}"))
}

#[cfg(unix)]
fn decode_response<Message>(frame: &DecodedFrame) -> Result<Message, DaemonError>
where
    Message: prost::Message + Default,
{
    frame.decode_message(frame.kind).map_err(protocol_error)
}

#[cfg(unix)]
fn protocol_status(protocol: Option<v1::ProtocolVersion>) -> Result<ProtocolStatus, DaemonError> {
    let protocol = protocol.ok_or_else(|| malformed("local response omitted protocol"))?;
    Ok(ProtocolStatus {
        wire_major: protocol.wire_major,
        state_schema: protocol.state_schema,
        capabilities: protocol.capabilities,
    })
}

#[cfg(unix)]
fn network_observation(
    response: &v1::LocalStatusResponse,
    device_id: zterm_core::DeviceId,
) -> Result<NetworkObservation, DaemonError> {
    let state = match response.network_state.as_str() {
        "" | "disabled" => NetworkState::Disabled,
        "initializing" => NetworkState::Initializing,
        "bound" => NetworkState::Bound,
        "degraded" => NetworkState::Degraded,
        "online" => NetworkState::Online,
        "stopping" => NetworkState::Stopping,
        "stopped" => NetworkState::Stopped,
        _ => return Err(malformed("status response contained unknown network state")),
    };
    let publish = address_service_state(&response.address_publish_state)?;
    let lookup = address_service_state(&response.address_lookup_state)?;
    let diagnostic = match response.network_diagnostic.as_str() {
        "" => None,
        "endpoint_bind_failed" => Some(NetworkDiagnostic::EndpointBindFailed),
        "endpoint_closed" => Some(NetworkDiagnostic::EndpointClosed),
        "home_relay_unavailable" => Some(NetworkDiagnostic::HomeRelayUnavailable),
        _ => {
            return Err(malformed(
                "status response contained unknown network diagnostic",
            ));
        }
    };
    Ok(NetworkObservation {
        device_id,
        state,
        endpoint_bound: response.endpoint_bound,
        bind_attempts: response.network_bind_attempts,
        home_relay: (!response.home_relay.is_empty()).then(|| response.home_relay.clone()),
        publish,
        lookup,
        authenticated_connection_count: response.authenticated_connection_count,
        primary_connection_count: response.primary_connection_count,
        active_stream_count: response.active_stream_count,
        direct_path_count: response.direct_path_count,
        relay_path_count: response.relay_path_count,
        diagnostic,
    })
}

#[cfg(unix)]
fn address_service_state(value: &str) -> Result<AddressServiceState, DaemonError> {
    match value {
        "" | "disabled" => Ok(AddressServiceState::Disabled),
        "configured" => Ok(AddressServiceState::Configured),
        "degraded" => Ok(AddressServiceState::Degraded),
        _ => Err(malformed(
            "status response contained unknown address-service state",
        )),
    }
}

#[cfg(unix)]
fn mutate_response(frame: DecodedFrame) -> Result<crate::session::SessionSummary, DaemonError> {
    let response: v1::SessionMutateResponse = decode_response(&frame)?;
    session_summary_from_wire(
        response
            .session
            .ok_or_else(|| malformed("session mutation response omitted session"))?,
    )
}

#[cfg(unix)]
fn resolved_target_wire(target: ResolvedSessionTarget) -> v1::TargetSelector {
    let target = match target.device_id() {
        Some(device_id) => v1::target_selector::Target::Device(device_id.into()),
        None => v1::target_selector::Target::Local(true),
    };
    v1::TargetSelector {
        target: Some(target),
    }
}

#[cfg(unix)]
fn resolved_target_from_wire(
    target: Option<v1::TargetSelector>,
) -> Result<ResolvedSessionTarget, DaemonError> {
    match target.and_then(|target| target.target) {
        Some(v1::target_selector::Target::Local(true)) => Ok(ResolvedSessionTarget::local()),
        Some(v1::target_selector::Target::Device(device_id)) => {
            let device_id = device_id.try_into().map_err(protocol_error)?;
            Ok(ResolvedSessionTarget::device(device_id))
        }
        _ => Err(malformed(
            "target resolution response omitted a valid frozen target",
        )),
    }
}

#[cfg(unix)]
fn connect_error(error: std::io::Error) -> DaemonError {
    let kind = match error.kind() {
        std::io::ErrorKind::PermissionDenied => DomainErrorKind::PermissionMismatch,
        _ => DomainErrorKind::DaemonStopped,
    };
    DaemonError::new(kind, format!("local daemon is unavailable: {error}"))
}

#[cfg(unix)]
fn daemon_io(operation: &str, error: std::io::Error) -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DaemonStopped,
        format!("{operation}: {error}"),
    )
}

#[cfg(unix)]
fn create_main_outcome_unknown() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::OperationOutcomeUnknown,
        "the default Session may have been created, but no complete correlated initial attachment result was received",
    )
}

#[cfg(unix)]
fn malformed(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

#[cfg(unix)]
fn resource_error(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::ResourceExhausted, detail)
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
    use std::time::Duration;

    use zterm_core::DaemonIncarnation;
    use zterm_core::terminal::MAX_HISTORY_WINDOW_ROWS;

    use super::*;

    #[test]
    fn live_scroll_metrics_match_enclosing_snapshot_and_delta_revisions() {
        let main = v1::TerminalActiveScreen::Main as i32;
        let metrics = v1::TerminalScrollMetrics {
            epoch: 3,
            revision: 7,
            offset_from_bottom: 0,
            max_offset_from_bottom: 12,
            viewport_rows: 24,
        };
        assert!(validate_live_scroll_metrics(Some(&metrics), 24, 7, main).is_ok());

        let mut invalid = metrics;
        invalid.offset_from_bottom = 1;
        assert_eq!(
            validate_live_scroll_metrics(Some(&invalid), 24, 7, main)
                .expect_err("snapshot metrics must describe live bottom")
                .kind(),
            DomainErrorKind::MalformedFrame
        );
        invalid = metrics;
        invalid.revision = 8;
        assert!(validate_live_scroll_metrics(Some(&invalid), 24, 7, main).is_err());
        invalid = metrics;
        invalid.epoch = 8;
        assert!(validate_live_scroll_metrics(Some(&invalid), 24, 7, main).is_err());
        assert!(
            validate_live_scroll_metrics(
                Some(&metrics),
                24,
                7,
                v1::TerminalActiveScreen::Alternate as i32,
            )
            .is_err()
        );
        assert!(
            validate_live_scroll_metrics(None, 24, 7, v1::TerminalActiveScreen::Alternate as i32,)
                .is_ok()
        );
    }

    #[test]
    fn viewport_frames_bind_to_current_height_product_bounds_and_monotonic_revision() {
        let mut frame = v1::TerminalViewportFrame {
            attachment_id: None,
            outcome: v1::TerminalViewportOutcome::Frame as i32,
            disposition: v1::TerminalViewportDisposition::Exact as i32,
            metrics: Some(v1::TerminalScrollMetrics {
                epoch: 3,
                revision: 7,
                offset_from_bottom: 2,
                max_offset_from_bottom: 12,
                viewport_rows: 24,
            }),
            rows: vec![b"row".to_vec(); 24],
            current_epoch: 3,
            current_revision: 7,
        };
        assert!(validate_viewport_frame(&frame, 24).is_ok());
        assert!(validate_viewport_frame(&frame, 23).is_err());

        frame.metrics.as_mut().expect("metrics").viewport_rows = 81;
        frame.rows.resize(81, b"row".to_vec());
        assert!(validate_viewport_frame(&frame, 81).is_err());

        frame.outcome = v1::TerminalViewportOutcome::Gap as i32;
        frame.disposition = v1::TerminalViewportDisposition::Unspecified as i32;
        frame.metrics = None;
        frame.rows.clear();
        frame.current_epoch = 8;
        frame.current_revision = 7;
        assert!(validate_viewport_frame(&frame, 24).is_err());
        frame.current_epoch = 0;
        frame.current_revision = 0;
        assert!(validate_viewport_frame(&frame, 24).is_ok());
    }

    #[test]
    fn history_window_frames_enforce_full_range_row_and_product_bounds() {
        let query = TerminalHistoryWindowQuery {
            anchor: TerminalHistoryWindowAnchor {
                epoch: Revision::new(3),
                revision: Revision::new(7),
                max_offset_from_bottom: 8,
                viewport: TerminalSize::new(4, 10),
            },
            target_offset_from_bottom: 3,
            older_margin_rows: 3,
            newer_margin_rows: 3,
        };
        let mut frame = v1::TerminalHistoryWindowFrame {
            attachment_id: None,
            outcome: v1::TerminalHistoryWindowOutcome::Frame as i32,
            disposition: v1::TerminalViewportDisposition::Exact as i32,
            anchor: Some(v1::TerminalHistoryWindowAnchor {
                epoch: 3,
                revision: 7,
                max_offset_from_bottom: 8,
                viewport_rows: 4,
                viewport_columns: 10,
            }),
            target_offset_from_bottom: 3,
            first_row_from_live_top: -6,
            ansi_rows: vec![b"row".to_vec(); 10],
            current_epoch: 3,
            current_revision: 7,
        };
        assert!(validate_history_window_frame(&frame, query).is_ok());

        frame.first_row_from_live_top = -9;
        assert!(validate_history_window_frame(&frame, query).is_err());
        frame.first_row_from_live_top = -6;
        frame
            .ansi_rows
            .resize(MAX_HISTORY_WINDOW_ROWS + 1, Vec::new());
        assert!(validate_history_window_frame(&frame, query).is_err());
        frame.ansi_rows.resize(10, b"row".to_vec());
        frame.anchor.as_mut().expect("anchor").viewport_columns = 241;
        assert!(validate_history_window_frame(&frame, query).is_err());
        frame.anchor.as_mut().expect("anchor").viewport_columns = 10;
        frame.anchor.as_mut().expect("anchor").revision = 6;
        frame.current_revision = 6;
        assert!(validate_history_window_frame(&frame, query).is_err());
    }

    #[tokio::test]
    async fn local_viewport_client_allows_only_one_outstanding_request() {
        let (mut client, _peer) = LocalAttachmentClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            SessionId::from_array([0x41; 16]),
            AttachmentId::from_array([0x42; 16]),
        );
        client
            .request_viewport(TerminalScrollAction::ScrollByLines(3))
            .await
            .expect("first semantic viewport request");
        let error = client
            .request_viewport(TerminalScrollAction::ScrollByLines(3))
            .await
            .expect_err("second request must be bounded until correlation");
        assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);
    }

    #[tokio::test]
    async fn local_history_window_client_allows_only_one_outstanding_request() {
        let (mut client, _peer) = LocalAttachmentClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            SessionId::from_array([0x43; 16]),
            AttachmentId::from_array([0x44; 16]),
        );
        let query = TerminalHistoryWindowQuery {
            anchor: zterm_core::terminal::TerminalHistoryWindowAnchor {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                max_offset_from_bottom: 12,
                viewport: zterm_core::terminal::TerminalSize::new(4, 10),
            },
            target_offset_from_bottom: 1,
            older_margin_rows: 8,
            newer_margin_rows: 0,
        };
        client
            .request_history_window(query)
            .await
            .expect("first bounded window request");
        let error = client
            .request_history_window(query)
            .await
            .expect_err("second window request must await correlation");
        assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);
    }

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
    fn local_client_and_takeover_token_debug_redact_private_owners() {
        const SOCKET_SENTINEL: &str = "/private/tmp/LOCAL_SOCKET_SENTINEL_91c7/daemon.sock";
        const INCARNATION_SENTINEL: &[u8; 16] = b"TAKEOVER_TOKEN_1";
        const SESSION_SENTINEL: &[u8; 16] = b"SESSION_TOKEN_01";

        let client = LocalClient::new(SOCKET_SENTINEL);
        let retry_owner = LocalTakeoverRetryToken {
            operation_id: OperationId {
                lease: OperationLease {
                    daemon_incarnation: DaemonIncarnation::from_array(*INCARNATION_SENTINEL),
                    ordinal: 8_675_309,
                },
                sequence: 2_434_117,
            },
            session_id: SessionId::from_array(*SESSION_SENTINEL),
        };
        let mutation_state = LocalMutationState {
            lease: Some(retry_owner.operation_id.lease),
            next_sequence: retry_owner.operation_id.sequence,
        };
        let rendered = format!("{client:?} {retry_owner:?} {mutation_state:?}");

        for sentinel in [
            SOCKET_SENTINEL,
            std::str::from_utf8(INCARNATION_SENTINEL).expect("ASCII sentinel"),
            std::str::from_utf8(SESSION_SENTINEL).expect("ASCII sentinel"),
            "8675309",
            "2434117",
        ] {
            assert!(!rendered.contains(sentinel));
        }
        assert!(rendered.contains("socket: \"[REDACTED]\""));
        assert!(rendered.contains("mutation_target_count: Some(1)"));
        assert!(rendered.contains("LocalTakeoverRetryToken([REDACTED])"));
        assert!(rendered.contains("has_lease: true"));
        assert_eq!(client.socket(), Path::new(SOCKET_SENTINEL));
        assert_eq!(
            retry_owner.operation_id.lease.daemon_incarnation.as_bytes(),
            INCARNATION_SENTINEL
        );
        assert_eq!(retry_owner.operation_id.lease.ordinal, 8_675_309);
        assert_eq!(retry_owner.operation_id.sequence, 2_434_117);
        assert_eq!(retry_owner.session_id.as_bytes(), SESSION_SENTINEL);
        assert_eq!(mutation_state.lease, Some(retry_owner.operation_id.lease));
        assert_eq!(mutation_state.next_sequence, 2_434_117);
    }

    #[test]
    fn local_session_end_debug_never_formats_the_signal_text() {
        let event = LocalAttachmentEvent::SessionEnded(v1::TerminalSessionEnded {
            session_id: Some(SessionId::from_array([0x90; SessionId::LENGTH]).into()),
            attachment_id: Some(AttachmentId::from_array([0x91; AttachmentId::LENGTH]).into()),
            reason: v1::TerminalSessionEndReason::NaturalExit as i32,
            exit_code: 1,
            signal: "SENSITIVE_LOCAL_SIGNAL_SENTINEL".to_owned(),
        });
        let debug = format!("{event:?}");
        assert!(debug.contains("has_signal: true"));
        assert!(!debug.contains("SENSITIVE_LOCAL_SIGNAL_SENTINEL"));
    }

    #[test]
    fn terminal_first_frame_router_separates_local_and_exact_device_targets() {
        let target = DeviceId::from_array([0x91; DeviceId::LENGTH]);
        let local = decoded_attach_target(v1::TargetSelector {
            target: Some(v1::target_selector::Target::Local(true)),
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

        let false_local = decoded_attach_target(v1::TargetSelector {
            target: Some(v1::target_selector::Target::Local(false)),
        });
        assert_eq!(
            terminal_attach_target(&false_local)
                .expect_err("false local selector is not a routing target")
                .kind(),
            DomainErrorKind::MalformedFrame
        );
    }

    #[tokio::test]
    async fn local_attachment_consumes_validated_transport_state_over_unix_duplex() {
        let session_id = SessionId::from_array([0x92; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0x93; AttachmentId::LENGTH]);
        let (mut client, mut daemon_stream) = LocalAttachmentClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        let event = v1::TerminalTransportStateEvent {
            attachment_id: Some(attachment_id.into()),
            state: v1::TerminalTransportState::Reconnecting as i32,
        };
        daemon_stream
            .write_all(
                &encode_message(WireKind::TerminalTransportStateEvent, 0, 0, &event)
                    .expect("bounded transport-state event"),
            )
            .await
            .expect("write transport-state event");
        match client
            .read_event(Duration::from_secs(1))
            .await
            .expect("validated transport-state event")
        {
            LocalAttachmentEvent::TransportState(actual) => assert_eq!(actual, event),
            event => panic!("unexpected local attachment event: {event:?}"),
        }

        let invalid = v1::TerminalTransportStateEvent {
            attachment_id: Some(attachment_id.into()),
            state: i32::MAX,
        };
        daemon_stream
            .write_all(
                &encode_message(WireKind::TerminalTransportStateEvent, 0, 0, &invalid)
                    .expect("bounded invalid transport-state event"),
            )
            .await
            .expect("write invalid transport-state event");
        assert_eq!(
            client
                .read_event(Duration::from_secs(1))
                .await
                .expect_err("unknown transport state is rejected")
                .kind(),
            DomainErrorKind::MalformedFrame
        );
    }

    #[tokio::test]
    async fn local_attachment_discards_stale_pre_snapshot_transport_states() {
        let temporary = tempfile::tempdir().expect("temporary socket root");
        let socket_path = temporary.path().join("terminal.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).expect("local listener");
        let session_id = SessionId::from_array([0x94; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0x95; AttachmentId::LENGTH]);
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept local view");
            let states = [
                v1::TerminalTransportState::Preparing,
                v1::TerminalTransportState::Synchronizing,
            ];
            for state in states {
                stream
                    .write_all(
                        &encode_message(
                            WireKind::TerminalTransportStateEvent,
                            0,
                            0,
                            &v1::TerminalTransportStateEvent {
                                attachment_id: Some(attachment_id.into()),
                                state: state as i32,
                            },
                        )
                        .expect("bounded pre-snapshot state"),
                    )
                    .await
                    .expect("write pre-snapshot state");
            }
            stream
                .write_all(
                    &encode_message(
                        WireKind::TerminalSnapshot,
                        1,
                        0,
                        &v1::TerminalSnapshot {
                            session_id: Some(session_id.into()),
                            attachment_id: Some(attachment_id.into()),
                            revision: 1,
                            rows: 24,
                            columns: 80,
                            screen_ansi: Vec::new(),
                            recent_history_ansi: Vec::new(),
                            active_screen: v1::TerminalActiveScreen::Main as i32,
                            modes: Some(v1::TerminalModes::default()),
                            scroll_metrics: None,
                        },
                    )
                    .expect("bounded initial snapshot"),
                )
                .await
                .expect("write initial snapshot");
            stream
                .write_all(
                    &encode_message(
                        WireKind::TerminalTransportStateEvent,
                        0,
                        0,
                        &v1::TerminalTransportStateEvent {
                            attachment_id: Some(attachment_id.into()),
                            state: v1::TerminalTransportState::Active as i32,
                        },
                    )
                    .expect("bounded post-snapshot state"),
                )
                .await
                .expect("write post-snapshot state");
        });

        let mut client = LocalAttachmentClient::connect_resolved(
            &socket_path,
            ResolvedSessionTarget::local(),
            None,
            true,
            false,
            None,
        )
        .await
        .expect("connect through pre-snapshot states");
        assert_eq!(client.initial_snapshot().revision, 1);
        let event = client
            .read_event(Duration::from_secs(1))
            .await
            .expect("post-snapshot state");
        assert!(matches!(
            event,
            LocalAttachmentEvent::TransportState(state)
                if state.state == v1::TerminalTransportState::Active as i32
        ));
        server.await.expect("server task");
    }

    #[tokio::test]
    async fn create_main_post_submit_response_loss_is_outcome_unknown_for_every_target() {
        let remote = DeviceId::from_array([0x96; DeviceId::LENGTH]);
        for target in [
            ResolvedSessionTarget::local(),
            ResolvedSessionTarget::device(remote),
        ] {
            let error = run_fake_create_main(target, FakeCreateMainReply::DropAfterSubmit)
                .await
                .expect_err("post-submit response loss has an unknown create outcome");
            assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
        }
    }

    #[tokio::test]
    async fn create_main_preserves_exact_snapshot_and_correlated_error_for_every_target() {
        let remote = DeviceId::from_array([0x97; DeviceId::LENGTH]);
        for (index, target) in [
            ResolvedSessionTarget::local(),
            ResolvedSessionTarget::device(remote),
        ]
        .into_iter()
        .enumerate()
        {
            let session_id = SessionId::from_array(
                [0x98 + u8::try_from(index).expect("two target fixtures fit u8");
                    SessionId::LENGTH],
            );
            let attachment_id = AttachmentId::from_array(
                [0xa8 + u8::try_from(index).expect("two target fixtures fit u8");
                    AttachmentId::LENGTH],
            );
            let client = run_fake_create_main(
                target,
                FakeCreateMainReply::Snapshot {
                    session_id,
                    attachment_id,
                },
            )
            .await
            .expect("a complete correlated snapshot is an exact committed result");
            assert_eq!(client.session_id(), session_id);
            assert_eq!(client.attachment_id(), attachment_id);
            assert_eq!(client.is_remote(), !target.is_local());

            let error = run_fake_create_main(
                target,
                FakeCreateMainReply::ServiceError {
                    request_id: 1,
                    kind: DomainErrorKind::SessionOccupied,
                    detail: "exact occupied fixture",
                },
            )
            .await
            .expect_err("a correlated typed service error is definitive");
            assert_eq!(error.kind(), DomainErrorKind::SessionOccupied);
            assert_eq!(error.detail(), "exact occupied fixture");
        }
    }

    #[tokio::test]
    async fn create_main_prewrite_failure_stays_definitive_and_wrong_error_id_is_unknown() {
        let temporary = tempfile::tempdir().expect("temporary missing socket root");
        let remote = DeviceId::from_array([0x9a; DeviceId::LENGTH]);
        for target in [
            ResolvedSessionTarget::local(),
            ResolvedSessionTarget::device(remote),
        ] {
            let error = LocalAttachmentClient::connect_resolved(
                temporary.path().join("missing.sock"),
                target,
                None,
                true,
                false,
                None,
            )
            .await
            .expect_err("connect failure occurs before any request write");
            assert_eq!(error.kind(), DomainErrorKind::DaemonStopped);

            let error = run_fake_create_main(
                target,
                FakeCreateMainReply::ServiceError {
                    request_id: 2,
                    kind: DomainErrorKind::SessionOccupied,
                    detail: "uncorrelated occupied fixture",
                },
            )
            .await
            .expect_err("an uncorrelated service error cannot prove the create outcome");
            assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn initial_attachment_deadline_covers_states_before_the_snapshot() {
        let create_main = run_fake_initial_state_stall(true).await;
        assert_eq!(
            create_main.kind(),
            DomainErrorKind::OperationOutcomeUnknown,
            "a stalled post-write create cannot be reported as definitively absent"
        );

        let existing = run_fake_initial_state_stall(false).await;
        assert_eq!(
            existing.kind(),
            DomainErrorKind::DeadlineExceeded,
            "an existing-session attach retains its bounded transport failure"
        );
    }

    #[tokio::test]
    async fn remote_mutation_outer_malformed_or_truncated_reply_sends_once() {
        let request_id = 501;
        let target = DeviceId::from_array([0xa1; DeviceId::LENGTH]);
        let mut truncated = encode_message(
            WireKind::SessionMutateResponse,
            request_id,
            0,
            &v1::SessionMutateResponse {
                session: Some(fake_session_summary(0xa1)),
            },
        )
        .expect("bounded mutation response");
        truncated.pop().expect("truncate local reply");

        for response in [vec![0x80, 0x00], truncated] {
            assert_remote_mutation_outer_failure_sends_once(request_id, target, response).await;
        }
    }

    #[tokio::test]
    async fn remote_mutation_outer_wrong_kind_or_request_id_sends_once() {
        let request_id = 511;
        let target = DeviceId::from_array([0xa2; DeviceId::LENGTH]);
        let wrong_kind = encode_message(
            WireKind::SessionListResponse,
            request_id,
            0,
            &v1::SessionListResponse { sessions: vec![] },
        )
        .expect("bounded wrong-kind reply");
        let wrong_id = encode_message(
            WireKind::SessionMutateResponse,
            request_id + 1,
            0,
            &v1::SessionMutateResponse {
                session: Some(fake_session_summary(0xa2)),
            },
        )
        .expect("bounded wrong-ID reply");

        for response in [wrong_kind, wrong_id] {
            assert_remote_mutation_outer_failure_sends_once(request_id, target, response).await;
        }
    }

    #[tokio::test]
    async fn remote_mutation_outer_invalid_typed_payload_sends_once() {
        let request_id = 516;
        let target = DeviceId::from_array([0xa4; DeviceId::LENGTH]);
        let missing_session = encode_message(
            WireKind::SessionMutateResponse,
            request_id,
            0,
            &v1::SessionMutateResponse { session: None },
        )
        .expect("well-framed incomplete mutation response");
        let unknown_error_code = encode_message(
            WireKind::ServiceErrorResponse,
            request_id,
            0,
            &v1::ServiceError {
                code: "unknown_remote_error".to_owned(),
                message: "invalid typed error fixture".to_owned(),
            },
        )
        .expect("well-framed invalid typed service error");

        for response in [missing_session, unknown_error_code] {
            assert_remote_mutation_outer_failure_sends_once(request_id, target, response).await;
        }
    }

    #[tokio::test]
    async fn remote_read_only_outer_post_write_failure_retries_once_but_prewrite_does_not() {
        let temporary = tempfile::tempdir().expect("temporary outer read fixture");
        let socket = temporary.path().join("outer-read.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind fake local daemon");
        let request_id = 521;
        let target = DeviceId::from_array([0xa3; DeviceId::LENGTH]);
        let response = encode_message(
            WireKind::SessionListResponse,
            request_id,
            0,
            &v1::SessionListResponse { sessions: vec![] },
        )
        .expect("bounded list reply");
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.expect("accept first outer request");
            let first_bytes = read_fake_unary(&mut first).await;
            first
                .write_all(&[0x80, 0x00])
                .await
                .expect("write malformed first reply");
            first.shutdown().await.expect("finish malformed reply");

            let (mut second, _) = listener.accept().await.expect("accept safe replay");
            let second_bytes = read_fake_unary(&mut second).await;
            second.write_all(&response).await.expect("write list reply");
            second.shutdown().await.expect("finish list reply");
            (first_bytes, second_bytes)
        });

        let client = LocalClient::new(&socket);
        let inner = encode_message(
            WireKind::SessionListRequest,
            request_id,
            1_000,
            &v1::SessionListRequest {
                target: Some(resolved_target_wire(ResolvedSessionTarget::device(target))),
            },
        )
        .expect("bounded list request");
        let frame = client
            .request_remote_preencoded(
                target,
                &inner,
                request_id,
                WireKind::SessionListResponse,
                Duration::from_secs(1),
                LocalRemoteRequestClass::ReadOnly,
            )
            .await
            .expect("safe read-only request retries one unresolved local reply");
        assert_eq!(frame.kind, WireKind::SessionListResponse);
        let (first, second) = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("fake local daemon completed")
            .expect("fake local daemon task");
        assert_eq!(first, second, "safe retry preserves exact envelope bytes");

        let missing_socket = temporary.path().join("missing.sock");
        let prewrite = LocalClient::new(missing_socket)
            .request_remote_preencoded(
                target,
                &inner,
                request_id,
                WireKind::SessionListResponse,
                Duration::from_secs(1),
                LocalRemoteRequestClass::ReadOnly,
            )
            .await
            .expect_err("pre-connect failure remains typed without ambiguity projection");
        assert_eq!(prewrite.kind(), DomainErrorKind::DaemonStopped);
    }

    #[tokio::test]
    async fn remote_operation_lease_outer_post_write_failure_sends_once() {
        let request_id = 526;
        let target = DeviceId::from_array([0xa5; DeviceId::LENGTH]);
        let inner = encode_message(
            WireKind::SessionOperationLeaseRequest,
            request_id,
            1_000,
            &v1::SessionOperationLeaseRequest {
                target: Some(resolved_target_wire(ResolvedSessionTarget::device(target))),
            },
        )
        .expect("bounded remote lease request");
        let valid_second_response = encode_message(
            WireKind::SessionOperationLeaseResponse,
            request_id,
            0,
            &v1::SessionOperationLeaseResponse {
                lease: Some(v1::OperationLease {
                    daemon_incarnation: vec![5; DaemonIncarnation::LENGTH],
                    ordinal: 9,
                }),
            },
        )
        .expect("bounded fallback lease response");
        let (result, requests) = run_remote_outer_failure(
            request_id,
            target,
            inner,
            WireKind::SessionOperationLeaseResponse,
            LocalRemoteRequestClass::StatefulControl,
            vec![0x80, 0x00],
            valid_second_response,
        )
        .await;

        assert_eq!(
            result
                .expect_err("stateful lease allocation is not an outer read-only retry")
                .kind(),
            DomainErrorKind::MalformedFrame
        );
        assert_eq!(
            requests.len(),
            1,
            "the outer lease-allocation envelope must not add a second retry layer"
        );
    }

    #[derive(Clone, Copy)]
    enum FakeCreateMainReply {
        DropAfterSubmit,
        ServiceError {
            request_id: u64,
            kind: DomainErrorKind,
            detail: &'static str,
        },
        Snapshot {
            session_id: SessionId,
            attachment_id: AttachmentId,
        },
    }

    async fn run_fake_initial_state_stall(create_main: bool) -> DaemonError {
        let temporary = tempfile::tempdir().expect("temporary initial-state stall fixture");
        let socket = temporary.path().join("initial-state-stall.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind fake local daemon");
        let attachment_id = AttachmentId::from_array([0x9b; AttachmentId::LENGTH]);
        let (state_written_sender, state_written_receiver) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept attach request");
            let first = read_first(&mut stream)
                .await
                .expect("decode complete attach request");
            assert_eq!(first.frame.kind, WireKind::TerminalAttachRequest);
            stream
                .write_all(
                    &encode_message(
                        WireKind::TerminalTransportStateEvent,
                        0,
                        0,
                        &v1::TerminalTransportStateEvent {
                            attachment_id: Some(attachment_id.into()),
                            state: v1::TerminalTransportState::Preparing as i32,
                        },
                    )
                    .expect("bounded pre-snapshot state"),
                )
                .await
                .expect("write pre-snapshot state");
            let _ = state_written_sender.send(());
            std::future::pending::<()>().await;
        });

        let client_socket = socket.clone();
        let client = tokio::spawn(async move {
            LocalAttachmentClient::connect_resolved(
                &client_socket,
                ResolvedSessionTarget::local(),
                None,
                create_main,
                false,
                None,
            )
            .await
        });
        state_written_receiver
            .await
            .expect("server crossed the complete pre-snapshot-state write barrier");
        let result = tokio::time::timeout(
            DEFAULT_DEADLINE
                .checked_mul(2)
                .expect("fixture watchdog deadline fits Duration"),
            client,
        )
        .await
        .expect("the production initial-response deadline fires before the fixture watchdog")
        .expect("initial-state client task")
        .expect_err("the initial snapshot never arrived");
        server.abort();
        let _ = server.await;
        result
    }

    async fn run_fake_create_main(
        target: ResolvedSessionTarget,
        reply: FakeCreateMainReply,
    ) -> Result<LocalAttachmentClient, DaemonError> {
        let temporary = tempfile::tempdir().expect("temporary create-main fixture");
        let socket = temporary.path().join("create-main.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind fake local daemon");
        let expected_target = target.device_id();
        let (submitted_tx, submitted_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept create-main request");
            let first = tokio::time::timeout(Duration::from_secs(2), read_first(&mut stream))
                .await
                .expect("create-main request reached the write boundary")
                .expect("decode create-main request");
            assert_eq!(first.frame.kind, WireKind::TerminalAttachRequest);
            assert_eq!(
                terminal_attach_target(&first.frame).expect("valid target"),
                expected_target
            );
            let request: v1::TerminalAttachRequest = first
                .frame
                .decode_message(WireKind::TerminalAttachRequest)
                .expect("decode create-main payload");
            assert!(request.create_main);
            submitted_tx
                .send(())
                .expect("observe request only after a complete frame was read");
            release_rx
                .await
                .expect("release fake response after submission barrier");

            match reply {
                FakeCreateMainReply::DropAfterSubmit => {}
                FakeCreateMainReply::ServiceError {
                    request_id,
                    kind,
                    detail,
                } => {
                    stream
                        .write_all(
                            &encode_message(
                                WireKind::ServiceErrorResponse,
                                request_id,
                                0,
                                &v1::ServiceError {
                                    code: kind.code().to_owned(),
                                    message: detail.to_owned(),
                                },
                            )
                            .expect("bounded typed create-main error"),
                        )
                        .await
                        .expect("write typed create-main error");
                }
                FakeCreateMainReply::Snapshot {
                    session_id,
                    attachment_id,
                } => {
                    stream
                        .write_all(
                            &encode_message(
                                WireKind::TerminalSnapshot,
                                1,
                                0,
                                &v1::TerminalSnapshot {
                                    session_id: Some(session_id.into()),
                                    attachment_id: Some(attachment_id.into()),
                                    revision: 1,
                                    rows: 24,
                                    columns: 80,
                                    screen_ansi: Vec::new(),
                                    recent_history_ansi: Vec::new(),
                                    active_screen: v1::TerminalActiveScreen::Main as i32,
                                    modes: Some(v1::TerminalModes::default()),
                                    scroll_metrics: None,
                                },
                            )
                            .expect("bounded committed create-main snapshot"),
                        )
                        .await
                        .expect("write committed create-main snapshot");
                }
            }
            let _ = stream.shutdown().await;
        });

        let client_socket = socket.clone();
        let client = tokio::spawn(async move {
            LocalAttachmentClient::connect_resolved(client_socket, target, None, true, false, None)
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), submitted_rx)
            .await
            .expect("create-main submission barrier is bounded")
            .expect("fake server reports create-main submission");
        release_tx
            .send(())
            .expect("release response-loss or exact-result fixture");
        let result = tokio::time::timeout(Duration::from_secs(2), client)
            .await
            .expect("create-main client completion is bounded")
            .expect("create-main client task");
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("fake create-main server completion is bounded")
            .expect("fake create-main server task");
        result
    }

    async fn assert_remote_mutation_outer_failure_sends_once(
        request_id: u64,
        target: DeviceId,
        first_response: Vec<u8>,
    ) {
        let inner = fake_remote_create_request(target, request_id);
        let valid_second_response = encode_message(
            WireKind::SessionMutateResponse,
            request_id,
            0,
            &v1::SessionMutateResponse {
                session: Some(fake_session_summary(0xaf)),
            },
        )
        .expect("bounded fallback response");
        let (result, requests) = run_remote_outer_failure(
            request_id,
            target,
            inner,
            WireKind::SessionMutateResponse,
            LocalRemoteRequestClass::Mutation,
            first_response,
            valid_second_response,
        )
        .await;
        let error = result.expect_err("an unresolved outer mutation reply is outcome unknown");
        assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
        assert_eq!(
            requests.len(),
            1,
            "the outer Unix mutation envelope must never be replayed after any bytes were written"
        );
    }

    async fn run_remote_outer_failure(
        request_id: u64,
        target: DeviceId,
        inner: Vec<u8>,
        response_kind: WireKind,
        request_class: LocalRemoteRequestClass,
        first_response: Vec<u8>,
        valid_second_response: Vec<u8>,
    ) -> (Result<DecodedFrame, DaemonError>, Vec<Vec<u8>>) {
        let temporary = tempfile::tempdir().expect("temporary outer one-send fixture");
        let socket = temporary.path().join("outer-one-send.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind fake local daemon");
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let (mut first, _) = listener.accept().await.expect("accept outer request");
            requests.push(read_fake_unary(&mut first).await);
            first
                .write_all(&first_response)
                .await
                .expect("write injected outer reply");
            let _ = first.shutdown().await;

            tokio::select! {
                _ = finished_rx => {}
                accepted = listener.accept() => {
                    let (mut replayed, _) = accepted.expect("accept unexpected outer replay");
                    requests.push(read_fake_unary(&mut replayed).await);
                    replayed
                        .write_all(&valid_second_response)
                        .await
                        .expect("write fallback response to unexpected replay");
                    let _ = replayed.shutdown().await;
                }
            }
            requests
        });

        let client = LocalClient::new(&socket);
        let result = client
            .request_remote_preencoded(
                target,
                &inner,
                request_id,
                response_kind,
                Duration::from_secs(1),
                request_class,
            )
            .await;
        let _ = finished_tx.send(());
        let requests = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("fake local daemon completed")
            .expect("fake local daemon task");
        (result, requests)
    }

    async fn read_fake_unary(stream: &mut tokio::net::UnixStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .await
            .expect("read fake local unary request");
        bytes
    }

    fn decoded_attach_target(target: v1::TargetSelector) -> DecodedFrame {
        let bytes = encode_message(
            WireKind::TerminalAttachRequest,
            1,
            0,
            &v1::TerminalAttachRequest {
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

    fn fake_remote_create_request(target: DeviceId, request_id: u64) -> Vec<u8> {
        encode_message(
            WireKind::SessionCreateRequest,
            request_id,
            1_000,
            &v1::SessionCreateRequest {
                operation_id: Some(
                    OperationId {
                        lease: OperationLease {
                            daemon_incarnation: DaemonIncarnation::from_array([4; 16]),
                            ordinal: 7,
                        },
                        sequence: 3,
                    }
                    .into(),
                ),
                target: Some(resolved_target_wire(ResolvedSessionTarget::device(target))),
                name: "outer-ambiguity".to_owned(),
                working_directory: String::new(),
                viewport: None,
            },
        )
        .expect("bounded remote mutation request")
    }

    fn fake_session_summary(byte: u8) -> v1::SessionSummary {
        v1::SessionSummary {
            session_id: Some(v1::SessionId {
                value: vec![byte; SessionId::LENGTH],
            }),
            name: "outer-ambiguity".to_owned(),
            revision: 2,
            has_controller: false,
            working_directory: "/tmp".to_owned(),
            viewport: Some(v1::TerminalViewport {
                rows: 24,
                columns: 80,
            }),
        }
    }

    #[tokio::test]
    async fn mutation_lease_state_is_isolated_and_serialized_only_per_exact_target() {
        let client = LocalClient::new("/unused/test.sock");
        let target_a = ResolvedSessionTarget::device(DeviceId::from_array([0xe1; 32]));
        let target_b = ResolvedSessionTarget::device(DeviceId::from_array([0xe2; 32]));
        let state_a = client
            .mutation_target_state(target_a)
            .expect("target A state");
        let state_b = client
            .mutation_target_state(target_b)
            .expect("target B state");
        {
            let mut a = state_a.lock().await;
            a.lease = Some(OperationLease {
                daemon_incarnation: DaemonIncarnation::from_array([1; 16]),
                ordinal: 11,
            });
            let mut b = state_b.lock().await;
            b.lease = Some(OperationLease {
                daemon_incarnation: DaemonIncarnation::from_array([2; 16]),
                ordinal: 22,
            });
        }

        let mut held_a = state_a.lock().await;
        held_a.lease = None;
        held_a.next_sequence = 1;
        let b = tokio::time::timeout(Duration::from_millis(100), state_b.lock())
            .await
            .expect("target B does not wait for target A's mutation lock");
        assert_eq!(b.lease.expect("target B lease retained").ordinal, 22);
        drop(b);
        drop(held_a);

        assert!(state_a.lock().await.lease.is_none());
        assert_eq!(
            state_b
                .lock()
                .await
                .lease
                .expect("target B remains unpoisoned")
                .ordinal,
            22
        );
    }

    #[test]
    fn mutation_target_cache_evicts_only_inactive_state_and_stays_hard_bounded() {
        let client = LocalClient::new("/unused/test.sock");
        let active_target =
            ResolvedSessionTarget::device(DeviceId::from_array([0xe1; DeviceId::LENGTH]));
        let active = client
            .mutation_target_state(active_target)
            .expect("active target state");

        for byte in 1_u8..=61 {
            client
                .mutation_target_state(ResolvedSessionTarget::device(DeviceId::from_array(
                    [byte; 32],
                )))
                .expect("bounded target slot");
        }
        client
            .mutation_target_state(ResolvedSessionTarget::device(DeviceId::from_array(
                [62; DeviceId::LENGTH],
            )))
            .expect("last bounded target slot");

        let replacement =
            ResolvedSessionTarget::device(DeviceId::from_array([0xfe; DeviceId::LENGTH]));
        client
            .mutation_target_state(replacement)
            .expect("inactive cached lease state is safely evicted");
        let states = client
            .mutation_targets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(states.len(), MAX_MUTATION_TARGETS_PER_CLIENT);
        assert!(Arc::ptr_eq(
            states
                .get(&active_target)
                .expect("externally retained state is never evicted"),
            &active
        ));
        drop(states);

        let saturated = LocalClient::new("/unused/saturated.sock");
        let mut active_states = vec![
            saturated
                .mutation_target_state(ResolvedSessionTarget::local())
                .expect("retain local target"),
        ];
        for index in 1..MAX_MUTATION_TARGETS_PER_CLIENT {
            let byte = u8::try_from(index).expect("test target index fits one byte");
            active_states.push(
                saturated
                    .mutation_target_state(ResolvedSessionTarget::device(DeviceId::from_array(
                        [byte; DeviceId::LENGTH],
                    )))
                    .expect("retain every bounded target slot"),
            );
        }
        assert_eq!(
            saturated
                .mutation_target_state(ResolvedSessionTarget::device(DeviceId::from_array(
                    [0xfe; DeviceId::LENGTH]
                )))
                .expect_err("in-flight target states cannot be evicted")
                .kind(),
            DomainErrorKind::ResourceExhausted
        );
        assert_eq!(
            saturated
                .mutation_targets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            MAX_MUTATION_TARGETS_PER_CLIENT
        );
        drop(active_states);
    }
}
