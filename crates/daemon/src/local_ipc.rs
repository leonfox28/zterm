//! Bounded same-UID unary and terminal IPC over Unix-domain sockets.

#[cfg(unix)]
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use iroh::SecretKey;
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::sync::{Mutex as AsyncMutex, Semaphore, mpsc, oneshot};
#[cfg(unix)]
use tokio::task::JoinSet;
#[cfg(unix)]
use zterm_core::{
    AttachmentId, OperationId, OperationLease, ResourceLimits, Revision, SessionSelector,
};
use zterm_core::{DomainErrorKind, SessionId, SessionName};
#[cfg(unix)]
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v1};

use crate::config::ValidatedConfig;
use crate::error::DaemonError;
use crate::service::{
    DaemonReadiness, DaemonService, DaemonStatus, SessionImpact, ValidatedSetupStatus,
};
#[cfg(unix)]
use crate::service::{ProtocolStatus, ServiceReply, protocol_error};
#[cfg(unix)]
use crate::session::{AttachmentLifecycle, AttachmentUpdate, SessionAttachment};

#[cfg(unix)]
const DEFAULT_DEADLINE: Duration = Duration::from_secs(5);
#[cfg(unix)]
const DRAIN_GRACE: Duration = Duration::from_secs(30);
#[cfg(unix)]
const ATTACHMENT_OUTBOUND_CAPACITY: usize = 8;

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
                            tracing::warn!(%error, "local listener accept failed; retrying");
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
        let drain = async { while handlers.join_next().await.is_some() {} };
        if tokio::time::timeout(DRAIN_GRACE, drain).await.is_err() {
            handlers.abort_all();
        }
    }
    fatal_error.map_or(Ok(()), Err)
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
    let frame = first.frame.clone();
    if frame.kind == WireKind::TerminalAttachRequest {
        let deadline = limits.request_deadline(frame.deadline_ms);
        let absolute_deadline = started + deadline;
        if let Err(error) =
            handle_attachment(stream, service, first, limits, absolute_deadline).await
        {
            tracing::debug!(error = %error, "local terminal attachment closed");
        }
        return;
    }
    let request_id = frame.request_id;
    let deadline = limits.request_deadline(frame.deadline_ms);
    let absolute_deadline = started + deadline;
    let remaining = deadline.saturating_sub(started.elapsed());
    let unary_finished = tokio::time::timeout(remaining, finish_unary(&mut stream, first)).await;
    if let Err(error) = unary_finished.unwrap_or_else(|_| {
        Err(DaemonError::new(
            DomainErrorKind::DeadlineExceeded,
            "local unary request did not finish before its deadline",
        ))
    }) {
        let _ = write_error(&mut stream, frame.request_id, &error).await;
        return;
    }
    let remaining = deadline.saturating_sub(started.elapsed());
    let reply =
        match tokio::time::timeout(remaining, service.dispatch_until(frame, absolute_deadline))
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
struct FirstFrame {
    frame: DecodedFrame,
    decoder: FrameDecoder,
    queued: VecDeque<DecodedFrame>,
}

#[cfg(unix)]
struct AttachmentOutbound {
    bytes: Vec<u8>,
    flushed: Option<oneshot::Sender<()>>,
}

#[cfg(unix)]
impl AttachmentOutbound {
    fn queued(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            flushed: None,
        }
    }
}

#[cfg(unix)]
async fn read_first(stream: &mut tokio::net::UnixStream) -> Result<FirstFrame, DaemonError> {
    let mut decoder = FrameDecoder::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|error| daemon_io("read local request", error))?;
        if read == 0 {
            decoder.finish().map_err(protocol_error)?;
            return Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                "local client closed before sending a request",
            ));
        }
        let mut frames = VecDeque::from(decoder.feed(&buffer[..read]).map_err(protocol_error)?);
        if let Some(frame) = frames.pop_front() {
            return Ok(FirstFrame {
                frame,
                decoder,
                queued: frames,
            });
        }
    }
}

#[cfg(unix)]
async fn finish_unary(
    stream: &mut tokio::net::UnixStream,
    mut first: FirstFrame,
) -> Result<(), DaemonError> {
    if !first.queued.is_empty() {
        return Err(malformed(
            "one unary connection may contain only one request",
        ));
    }
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|error| daemon_io("finish local unary request", error))?;
        if read == 0 {
            return first.decoder.finish().map_err(protocol_error);
        }
        if !first
            .decoder
            .feed(&buffer[..read])
            .map_err(protocol_error)?
            .is_empty()
        {
            return Err(malformed(
                "one unary connection may contain only one request",
            ));
        }
    }
}

#[cfg(unix)]
async fn handle_attachment(
    mut stream: tokio::net::UnixStream,
    service: Arc<DaemonService>,
    first: FirstFrame,
    limits: LocalIpcLimits,
    deadline: Instant,
) -> Result<(), DaemonError> {
    let prepare_service = Arc::clone(&service);
    let prepare_frame = first.frame.clone();
    let prepared = match run_blocking_until(deadline, move || {
        prepare_local_attachment(&prepare_service, &prepare_frame, deadline)
    })
    .await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            write_error(&mut stream, first.frame.request_id, &error)
                .await
                .map_err(|write| daemon_io("write terminal attach error", write))?;
            return Err(error);
        }
    };
    let attachment = prepared.attachment;
    let initial = encode_snapshot(
        first.frame.request_id,
        attachment.session_id(),
        attachment.attachment_id(),
        prepared.snapshot,
    )?;
    stream
        .write_all(&initial)
        .await
        .map_err(|error| daemon_io("write initial terminal snapshot", error))?;

    let view_id = local_view_id();
    let principal = service.sessions().local_principal(view_id);
    let (reader, writer) = stream.into_split();
    let (outbound_sender, outbound_receiver) =
        mpsc::channel::<AttachmentOutbound>(ATTACHMENT_OUTBOUND_CAPACITY);
    let reader_attachment = Arc::clone(&attachment);
    let reader_service = Arc::clone(&service);
    let mut reader_task = tokio::spawn(async move {
        attachment_reader(
            reader,
            first.decoder,
            first.queued,
            AttachmentReaderContext {
                service: reader_service,
                attachment: reader_attachment,
                principal,
                outbound: outbound_sender,
                limits,
            },
        )
        .await
    });
    let writer_attachment = Arc::clone(&attachment);
    let mut writer_task = tokio::spawn(async move {
        attachment_writer(writer, writer_attachment, outbound_receiver).await
    });

    let result = tokio::select! {
        result = &mut reader_task => {
            writer_task.abort();
            flatten_attachment_task(result)
        }
        result = &mut writer_task => {
            reader_task.abort();
            flatten_attachment_task(result)
        }
    };
    attachment.detach();
    result
}

#[cfg(unix)]
fn prepare_local_attachment(
    service: &DaemonService,
    frame: &DecodedFrame,
    deadline: Instant,
) -> Result<crate::session::PreparedAttachment, DaemonError> {
    let request: v1::TerminalAttachRequest = frame
        .decode_message(WireKind::TerminalAttachRequest)
        .map_err(protocol_error)?;
    require_local_target(request.target.clone())?;
    let (selector, create_main) = terminal_selector(&request)?;
    let viewport = request
        .viewport
        .map(TryInto::try_into)
        .transpose()
        .map_err(protocol_error)?;
    service.sessions().prepare_attach_until(
        selector,
        create_main,
        request.takeover,
        viewport,
        deadline,
    )
}

#[cfg(unix)]
fn flatten_attachment_task(
    result: Result<Result<(), DaemonError>, tokio::task::JoinError>,
) -> Result<(), DaemonError> {
    result.map_err(|error| {
        DaemonError::new(
            DomainErrorKind::Cancelled,
            format!("local attachment task ended unexpectedly: {error}"),
        )
    })?
}

#[cfg(unix)]
async fn run_blocking_until<T, F>(deadline: Instant, operation: F) -> Result<T, DaemonError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, DaemonError> + Send + 'static,
{
    let remaining = deadline.saturating_duration_since(Instant::now());
    match tokio::time::timeout(remaining, tokio::task::spawn_blocking(operation)).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            format!("blocking local session worker ended unexpectedly: {error}"),
        )),
        Err(_) => Err(DaemonError::new(
            DomainErrorKind::DeadlineExceeded,
            "local session operation exceeded its absolute deadline",
        )),
    }
}

#[cfg(unix)]
struct AttachmentReaderContext {
    service: Arc<DaemonService>,
    attachment: Arc<SessionAttachment>,
    principal: zterm_core::AttachmentPrincipal,
    outbound: mpsc::Sender<AttachmentOutbound>,
    limits: LocalIpcLimits,
}

#[cfg(unix)]
async fn attachment_reader(
    mut reader: tokio::net::unix::OwnedReadHalf,
    mut decoder: FrameDecoder,
    mut queued: VecDeque<DecodedFrame>,
    context: AttachmentReaderContext,
) -> Result<(), DaemonError> {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        if let Some(frame) = queued.pop_front() {
            match process_attachment_frame(
                frame.clone(),
                &context.service,
                &context.attachment,
                context.principal,
                &context.outbound,
                context.limits,
            )
            .await
            {
                Ok(false) => continue,
                Ok(true) => return Ok(()),
                Err(error) => {
                    flush_attachment_error(&context.outbound, frame.request_id, &error).await;
                    return Err(error);
                }
            }
        }

        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| daemon_io("read local terminal stream", error))?;
        if read == 0 {
            if let Err(error) = decoder.finish() {
                let error = protocol_error(error);
                flush_attachment_error(&context.outbound, 0, &error).await;
                return Err(error);
            }
            return Ok(());
        }
        match decoder.feed(&buffer[..read]) {
            Ok(frames) => queued.extend(frames),
            Err(error) => {
                let error = protocol_error(error);
                flush_attachment_error(&context.outbound, 0, &error).await;
                return Err(error);
            }
        }
    }
}

#[cfg(unix)]
async fn flush_attachment_error(
    outbound: &mpsc::Sender<AttachmentOutbound>,
    request_id: u64,
    error: &DaemonError,
) {
    let (flushed, wait_for_flush) = oneshot::channel();
    let flush = async {
        outbound
            .send(AttachmentOutbound {
                bytes: ServiceReply::error(request_id, error).bytes,
                flushed: Some(flushed),
            })
            .await
            .map_err(|_| ())?;
        wait_for_flush.await.map_err(|_| ())
    };
    let _ = tokio::time::timeout(DEFAULT_DEADLINE, flush).await;
}

#[cfg(unix)]
async fn process_attachment_frame(
    frame: DecodedFrame,
    service: &Arc<DaemonService>,
    attachment: &Arc<SessionAttachment>,
    principal: zterm_core::AttachmentPrincipal,
    outbound: &mpsc::Sender<AttachmentOutbound>,
    limits: LocalIpcLimits,
) -> Result<bool, DaemonError> {
    let deadline = Instant::now() + limits.request_deadline(frame.deadline_ms);
    match frame.kind {
        WireKind::TerminalSnapshotApplied => {
            let request: v1::TerminalSnapshotApplied = frame
                .decode_message(WireKind::TerminalSnapshotApplied)
                .map_err(protocol_error)?;
            require_attachment_id(request.attachment_id, attachment)?;
            let attachment_worker = Arc::clone(attachment);
            let revision = Revision::new(request.revision);
            let snapshot = run_blocking_until(deadline, move || {
                attachment_worker.snapshot_applied_until(revision, deadline)
            })
            .await?;
            if let Some(snapshot) = snapshot {
                send_resync(frame.request_id, attachment, snapshot, outbound).await?;
            }
            Ok(false)
        }
        WireKind::TerminalSyncRequest => {
            let request: v1::TerminalSyncRequest = frame
                .decode_message(WireKind::TerminalSyncRequest)
                .map_err(protocol_error)?;
            require_attachment_id(request.attachment_id, attachment)?;
            let attachment_worker = Arc::clone(attachment);
            let known_revision = Revision::new(request.known_revision);
            let snapshot = run_blocking_until(deadline, move || {
                attachment_worker.sync_latest_until(known_revision, deadline)
            })
            .await?;
            send_resync(frame.request_id, attachment, snapshot, outbound).await?;
            Ok(false)
        }
        WireKind::TerminalInput => {
            let request: v1::TerminalInput = frame
                .decode_message(WireKind::TerminalInput)
                .map_err(protocol_error)?;
            require_attachment_id(request.attachment_id, attachment)?;
            let attachment_worker = Arc::clone(attachment);
            run_blocking_until(deadline, move || {
                attachment_worker.write_input_until(&request.bytes, deadline)
            })
            .await?;
            Ok(false)
        }
        WireKind::TerminalResize => {
            let request: v1::TerminalResize = frame
                .decode_message(WireKind::TerminalResize)
                .map_err(protocol_error)?;
            require_attachment_id(request.attachment_id, attachment)?;
            let size = v1::TerminalViewport {
                rows: request.rows,
                columns: request.columns,
            }
            .try_into()
            .map_err(protocol_error)?;
            let attachment_worker = Arc::clone(attachment);
            run_blocking_until(deadline, move || {
                attachment_worker.resize_until(size, deadline)
            })
            .await?;
            Ok(false)
        }
        WireKind::TerminalDetach => {
            let request: v1::TerminalDetach = frame
                .decode_message(WireKind::TerminalDetach)
                .map_err(protocol_error)?;
            require_attachment_id(request.attachment_id, attachment)?;
            Ok(true)
        }
        WireKind::SessionOperationLeaseRequest => {
            let request: v1::SessionOperationLeaseRequest = frame
                .decode_message(WireKind::SessionOperationLeaseRequest)
                .map_err(protocol_error)?;
            require_local_target(request.target)?;
            let lease = service.sessions().issue_operation_lease(principal)?;
            outbound
                .send(AttachmentOutbound::queued(
                    encode_message(
                        WireKind::SessionOperationLeaseResponse,
                        frame.request_id,
                        0,
                        &v1::SessionOperationLeaseResponse {
                            lease: Some(lease.into()),
                        },
                    )
                    .map_err(protocol_error)?,
                ))
                .await
                .map_err(|_| attachment_cancelled())?;
            Ok(false)
        }
        WireKind::SessionTakeoverRequest => {
            let request: v1::SessionTakeoverRequest = frame
                .decode_message(WireKind::SessionTakeoverRequest)
                .map_err(protocol_error)?;
            require_local_target(request.target.clone())?;
            let session_id: SessionId = request
                .session_id
                .clone()
                .ok_or_else(|| malformed("takeover omitted session_id"))?
                .try_into()
                .map_err(protocol_error)?;
            if session_id != attachment.session_id() {
                return Err(malformed("takeover session_id does not match this stream"));
            }
            require_attachment_id(request.attachment_id.clone(), attachment)?;
            let operation_id = request
                .operation_id
                .ok_or_else(|| malformed("takeover omitted operation_id"))?
                .try_into()
                .map_err(protocol_error)?;
            let service_worker = Arc::clone(service);
            let attachment_worker = Arc::clone(attachment);
            let summary = run_blocking_until(deadline, move || {
                service_worker.sessions().takeover_until(
                    principal,
                    operation_id,
                    &attachment_worker,
                    deadline,
                )
            })
            .await?;
            let message = v1::SessionMutateResponse {
                session: Some(v1::SessionSummary {
                    session_id: Some(summary.session_id.into()),
                    name: summary.name.to_string(),
                    revision: summary.revision.get(),
                    has_controller: summary.has_controller,
                    working_directory: summary.working_directory.to_string_lossy().into_owned(),
                    viewport: Some(summary.viewport.into()),
                }),
            };
            outbound
                .send(AttachmentOutbound::queued(
                    encode_message(
                        WireKind::SessionMutateResponse,
                        frame.request_id,
                        0,
                        &message,
                    )
                    .map_err(protocol_error)?,
                ))
                .await
                .map_err(|_| attachment_cancelled())?;
            Ok(false)
        }
        _ => Err(malformed(format!(
            "wire kind {:?} is invalid on a terminal attachment",
            frame.kind
        ))),
    }
}

#[cfg(unix)]
async fn attachment_writer(
    mut writer: tokio::net::unix::OwnedWriteHalf,
    attachment: Arc<SessionAttachment>,
    mut outbound: mpsc::Receiver<AttachmentOutbound>,
) -> Result<(), DaemonError> {
    let mut revisions = attachment.revision_watch()?;
    let mut lifecycle = attachment.lifecycle_watch()?;
    let mut revisions_open = true;
    let initial_lifecycle = lifecycle.borrow().clone();
    if write_lifecycle_event(&mut writer, &attachment, initial_lifecycle).await? {
        return Ok(());
    }
    loop {
        tokio::select! {
            biased;
            message = outbound.recv() => {
                let Some(message) = message else {
                    return Ok(());
                };
                writer.write_all(&message.bytes).await.map_err(|error| daemon_io("write terminal response", error))?;
                if let Some(flushed) = message.flushed {
                    let _ = flushed.send(());
                }
            }
            changed = lifecycle.changed() => {
                changed.map_err(|_| attachment_cancelled())?;
                let event = lifecycle.borrow_and_update().clone();
                if write_lifecycle_event(&mut writer, &attachment, event).await? {
                    return Ok(());
                }
            }
            changed = revisions.changed(), if revisions_open => {
                if changed.is_err() {
                    // Driver finalization closes its revision watch before the actor publishes
                    // the final drained update and SessionEnded lifecycle value. Keep the stream
                    // alive for that authoritative terminal event instead of reporting a
                    // connection-local cancellation.
                    revisions_open = false;
                } else {
                    match attachment_next_update(Arc::clone(&attachment)).await {
                        Ok(Some(update)) => {
                            write_terminal_update(&mut writer, &attachment, update).await?;
                        }
                        Ok(None) => {}
                        Err(error) if error.kind() == DomainErrorKind::SessionNotFound => {
                            // A revision notification can race the actor's transition into final
                            // drain. The lifecycle channel owns the terminal event and final
                            // checkpoint, so stop polling revisions and wait for it.
                            revisions_open = false;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        }
    }
}

#[cfg(unix)]
async fn write_lifecycle_event(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    attachment: &Arc<SessionAttachment>,
    event: AttachmentLifecycle,
) -> Result<bool, DaemonError> {
    match event {
        AttachmentLifecycle::AwaitingSnapshot { .. } => Ok(false),
        AttachmentLifecycle::Active { .. } | AttachmentLifecycle::PreparedTakeover => {
            if let Some(update) = attachment_next_update(Arc::clone(attachment)).await? {
                write_terminal_update(writer, attachment, update).await?;
            }
            Ok(false)
        }
        AttachmentLifecycle::LeaseLost { generation } => {
            let message = v1::TerminalLeaseLost {
                attachment_id: Some(attachment.attachment_id().into()),
                generation,
            };
            let bytes = encode_message(WireKind::TerminalLeaseLost, 0, 0, &message)
                .map_err(protocol_error)?;
            writer
                .write_all(&bytes)
                .await
                .map_err(|error| daemon_io("write lease-lost event", error))?;
            Ok(true)
        }
        AttachmentLifecycle::SessionEnded(reason) => {
            let final_attachment = Arc::clone(attachment);
            let deadline = Instant::now() + DEFAULT_DEADLINE;
            if let Ok(Some(update)) =
                run_blocking_until(deadline, move || final_attachment.final_update()).await
            {
                write_terminal_update(writer, attachment, update).await?;
            }
            let message = session_ended_message(attachment, reason);
            let bytes = encode_message(WireKind::TerminalSessionEnded, 0, 0, &message)
                .map_err(protocol_error)?;
            writer
                .write_all(&bytes)
                .await
                .map_err(|error| daemon_io("write session-ended event", error))?;
            Ok(true)
        }
    }
}

#[cfg(unix)]
async fn attachment_next_update(
    attachment: Arc<SessionAttachment>,
) -> Result<Option<AttachmentUpdate>, DaemonError> {
    let deadline = Instant::now() + DEFAULT_DEADLINE;
    run_blocking_until(deadline, move || attachment.next_update_until(deadline)).await
}

#[cfg(unix)]
async fn write_terminal_update(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    attachment: &SessionAttachment,
    update: AttachmentUpdate,
) -> Result<(), DaemonError> {
    match update {
        AttachmentUpdate::Delta(delta) => {
            let message = zterm_proto::terminal_delta_message(delta);
            let bytes =
                encode_message(WireKind::TerminalDelta, 0, 0, &message).map_err(protocol_error)?;
            writer
                .write_all(&bytes)
                .await
                .map_err(|error| daemon_io("write terminal delta", error))
        }
        AttachmentUpdate::Snapshot(snapshot) => {
            let required = v1::TerminalSyncRequired {
                attachment_id: Some(attachment.attachment_id().into()),
                latest_revision: snapshot.revision.get(),
            };
            let required = encode_message(WireKind::TerminalSyncRequired, 0, 0, &required)
                .map_err(protocol_error)?;
            writer
                .write_all(&required)
                .await
                .map_err(|error| daemon_io("write terminal resync requirement", error))?;
            let snapshot = encode_snapshot(
                0,
                attachment.session_id(),
                attachment.attachment_id(),
                snapshot,
            )?;
            writer
                .write_all(&snapshot)
                .await
                .map_err(|error| daemon_io("write terminal resync snapshot", error))
        }
    }
}

#[cfg(unix)]
async fn send_resync(
    request_id: u64,
    attachment: &SessionAttachment,
    snapshot: zterm_core::terminal::TerminalSnapshot,
    outbound: &mpsc::Sender<AttachmentOutbound>,
) -> Result<(), DaemonError> {
    let required = v1::TerminalSyncRequired {
        attachment_id: Some(attachment.attachment_id().into()),
        latest_revision: snapshot.revision.get(),
    };
    outbound
        .send(AttachmentOutbound::queued(
            encode_message(WireKind::TerminalSyncRequired, request_id, 0, &required)
                .map_err(protocol_error)?,
        ))
        .await
        .map_err(|_| attachment_cancelled())?;
    outbound
        .send(AttachmentOutbound::queued(encode_snapshot(
            request_id,
            attachment.session_id(),
            attachment.attachment_id(),
            snapshot,
        )?))
        .await
        .map_err(|_| attachment_cancelled())
}

#[cfg(unix)]
fn encode_snapshot(
    request_id: u64,
    session_id: SessionId,
    attachment_id: AttachmentId,
    snapshot: zterm_core::terminal::TerminalSnapshot,
) -> Result<Vec<u8>, DaemonError> {
    let message = zterm_proto::terminal_snapshot_message(session_id, attachment_id, snapshot);
    encode_message(WireKind::TerminalSnapshot, request_id, 0, &message).map_err(protocol_error)
}

#[cfg(unix)]
fn terminal_selector(
    request: &v1::TerminalAttachRequest,
) -> Result<(Option<SessionSelector>, bool), DaemonError> {
    let has_id = request.session_id.is_some();
    let has_name = !request.session_name.is_empty();
    let selections = usize::from(has_id) + usize::from(has_name) + usize::from(request.create_main);
    if selections != 1 {
        return Err(malformed(
            "terminal attach requires exactly one session_id, session_name, or create_main",
        ));
    }
    if request.create_main {
        return Ok((None, true));
    }
    if let Some(session_id) = request.session_id.clone() {
        return Ok((
            Some(SessionSelector::Id(
                session_id.try_into().map_err(protocol_error)?,
            )),
            false,
        ));
    }
    let name = SessionName::new(request.session_name.clone()).map_err(|error| {
        DaemonError::new(DomainErrorKind::InvalidSessionName, error.to_string())
    })?;
    Ok((Some(SessionSelector::Name(name)), false))
}

#[cfg(unix)]
fn require_local_target(target: Option<v1::TargetSelector>) -> Result<(), DaemonError> {
    match target.and_then(|target| target.target) {
        Some(v1::target_selector::Target::Local(true)) => Ok(()),
        _ => Err(malformed(
            "local terminal stream requires target.local=true",
        )),
    }
}

#[cfg(unix)]
fn require_attachment_id(
    attachment_id: Option<v1::AttachmentId>,
    attachment: &SessionAttachment,
) -> Result<(), DaemonError> {
    let attachment_id: AttachmentId = attachment_id
        .ok_or_else(|| malformed("terminal message omitted attachment_id"))?
        .try_into()
        .map_err(protocol_error)?;
    if attachment_id == attachment.attachment_id() {
        Ok(())
    } else {
        Err(malformed(
            "terminal message attachment_id does not match this stream",
        ))
    }
}

#[cfg(unix)]
fn session_ended_message(
    attachment: &SessionAttachment,
    reason: zterm_core::SessionEndReason,
) -> v1::TerminalSessionEnded {
    let (reason, exit_code, signal) = match reason {
        zterm_core::SessionEndReason::NaturalExit { exit_code, signal } => (
            v1::TerminalSessionEndReason::NaturalExit,
            exit_code,
            signal.unwrap_or_default(),
        ),
        zterm_core::SessionEndReason::ExplicitClose => (
            v1::TerminalSessionEndReason::ExplicitClose,
            0,
            String::new(),
        ),
        zterm_core::SessionEndReason::DaemonStop => {
            (v1::TerminalSessionEndReason::DaemonStop, 0, String::new())
        }
        zterm_core::SessionEndReason::DriverFailure => (
            v1::TerminalSessionEndReason::DriverFailure,
            0,
            String::new(),
        ),
    };
    v1::TerminalSessionEnded {
        session_id: Some(attachment.session_id().into()),
        attachment_id: Some(attachment.attachment_id().into()),
        reason: reason as i32,
        exit_code,
        signal,
    }
}

#[cfg(unix)]
fn local_view_id() -> AttachmentId {
    let secret = SecretKey::generate().to_bytes();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&secret[..16]);
    AttachmentId::from_array(bytes)
}

/// One typed server message received on a local terminal attachment.
#[cfg(unix)]
#[derive(Clone, Debug, PartialEq)]
#[doc(hidden)]
pub enum LocalAttachmentEvent {
    /// A full host-authoritative replacement state.
    Snapshot(v1::TerminalSnapshot),
    /// A merged revision update from the acknowledged checkpoint.
    Delta(v1::TerminalDelta),
    /// The following snapshot must replace the client state atomically.
    SyncRequired(v1::TerminalSyncRequired),
    /// A prepared takeover committed successfully.
    Takeover(crate::session::SessionSummary),
    /// Another attachment replaced this controller.
    LeaseLost(v1::TerminalLeaseLost),
    /// The underlying session and PTY ended.
    SessionEnded(v1::TerminalSessionEnded),
}

/// Opaque same-daemon retry token for one takeover whose response was lost.
///
/// It is intentionally process-memory only in M4. Callers must export and
/// retain it explicitly if they need fresh-process ambiguity recovery.
#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct LocalTakeoverRetryToken {
    operation_id: OperationId,
    session_id: SessionId,
}

/// Real same-UID duplex socket adapter used before the final raw-terminal UI exists.
#[cfg(unix)]
#[derive(Debug)]
#[doc(hidden)]
pub struct LocalAttachmentClient {
    stream: tokio::net::UnixStream,
    decoder: FrameDecoder,
    queued: VecDeque<DecodedFrame>,
    deferred: VecDeque<DecodedFrame>,
    session_id: SessionId,
    attachment_id: AttachmentId,
    initial_snapshot: v1::TerminalSnapshot,
    next_request_id: u64,
    operation_lease: Option<OperationLease>,
    next_operation_sequence: u64,
}

#[cfg(unix)]
impl LocalAttachmentClient {
    /// Attaches to the daemon-lifetime default `main` session, creating it if absent.
    pub async fn connect_main(
        socket: impl AsRef<Path>,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(socket.as_ref(), None, true, false, viewport).await
    }

    /// Attaches to an existing session selected by stable ID or exact name.
    pub async fn connect_session(
        socket: impl AsRef<Path>,
        selector: SessionSelector,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(socket.as_ref(), Some(selector), false, takeover, viewport).await
    }

    async fn connect_inner(
        socket: &Path,
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
                target: Some(local_target()),
                session_id,
                takeover,
                session_name,
                create_main,
                viewport: viewport.map(Into::into),
            },
        )
        .map_err(protocol_error)?;
        let mut stream = tokio::net::UnixStream::connect(socket)
            .await
            .map_err(connect_error)?;
        stream
            .write_all(&bytes)
            .await
            .map_err(|error| daemon_io("write local attach request", error))?;
        let first = tokio::time::timeout(DEFAULT_DEADLINE, read_first(&mut stream))
            .await
            .map_err(|_| {
                DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    "timed out waiting for initial terminal snapshot",
                )
            })??;
        if first.frame.kind == WireKind::ServiceErrorResponse {
            return Err(service_error(&first.frame)?);
        }
        if first.frame.request_id != request_id {
            return Err(malformed("initial terminal snapshot request_id mismatch"));
        }
        let initial_snapshot: v1::TerminalSnapshot = first
            .frame
            .decode_message(WireKind::TerminalSnapshot)
            .map_err(protocol_error)?;
        let session_id = required_snapshot_session_id(&initial_snapshot)?;
        let attachment_id = required_snapshot_attachment_id(&initial_snapshot)?;
        validate_snapshot_viewport(&initial_snapshot)?;
        Ok(Self {
            stream,
            decoder: first.decoder,
            queued: first.queued,
            deferred: VecDeque::new(),
            session_id,
            attachment_id,
            initial_snapshot,
            next_request_id: request_id + 1,
            operation_lease: None,
            next_operation_sequence: 1,
        })
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

    /// Returns the initial full state which must be acknowledged before input.
    #[must_use]
    pub const fn initial_snapshot(&self) -> &v1::TerminalSnapshot {
        &self.initial_snapshot
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
    }

    /// Commits a previously prepared and acknowledged takeover attachment.
    pub async fn takeover(&mut self) -> Result<(), DaemonError> {
        self.begin_takeover().await.map(|_| ())
    }

    /// Sends a takeover and returns the opaque token required after ambiguous
    /// response loss.
    #[doc(hidden)]
    pub async fn begin_takeover(&mut self) -> Result<LocalTakeoverRetryToken, DaemonError> {
        let operation_id = self.next_operation_id().await?;
        let receipt = LocalTakeoverRetryToken {
            operation_id,
            session_id: self.session_id,
        };
        self.send_takeover(operation_id).await?;
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
        self.send_takeover(token.operation_id).await
    }

    async fn send_takeover(&mut self, operation_id: OperationId) -> Result<(), DaemonError> {
        self.send(
            WireKind::SessionTakeoverRequest,
            &v1::SessionTakeoverRequest {
                operation_id: Some(operation_id.into()),
                target: Some(local_target()),
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
        let frame = tokio::time::timeout(deadline, self.read_frame())
            .await
            .map_err(|_| {
                DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    "timed out waiting for local terminal event",
                )
            })??;
        if frame.kind == WireKind::ServiceErrorResponse {
            let error = service_error(&frame)?;
            if error.kind() == DomainErrorKind::OperationOutcomeUnknown {
                self.operation_lease = None;
                self.next_operation_sequence = 1;
            }
            return Err(error);
        }
        match frame.kind {
            WireKind::TerminalSnapshot => {
                let snapshot: v1::TerminalSnapshot = frame
                    .decode_message(WireKind::TerminalSnapshot)
                    .map_err(protocol_error)?;
                self.require_snapshot_identity(&snapshot)?;
                validate_snapshot_viewport(&snapshot)?;
                Ok(LocalAttachmentEvent::Snapshot(snapshot))
            }
            WireKind::TerminalDelta => Ok(LocalAttachmentEvent::Delta(
                frame
                    .decode_message(WireKind::TerminalDelta)
                    .map_err(protocol_error)?,
            )),
            WireKind::TerminalSyncRequired => {
                let required: v1::TerminalSyncRequired = frame
                    .decode_message(WireKind::TerminalSyncRequired)
                    .map_err(protocol_error)?;
                self.require_attachment(required.attachment_id.clone())?;
                Ok(LocalAttachmentEvent::SyncRequired(required))
            }
            WireKind::SessionMutateResponse => {
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
            .map_err(|error| daemon_io("finish local terminal detach", error))
    }

    async fn send<Message: prost::Message>(
        &mut self,
        kind: WireKind,
        message: &Message,
    ) -> Result<(), DaemonError> {
        let request_id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or_else(|| resource_error("local attachment request ID exhausted"))?;
        let bytes = encode_message(kind, request_id, 0, message).map_err(protocol_error)?;
        self.stream
            .write_all(&bytes)
            .await
            .map_err(|error| daemon_io("write local terminal message", error))
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
                .map_err(|error| daemon_io("read local terminal event", error))?;
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
                    target: Some(local_target()),
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
    let _: zterm_core::terminal::TerminalSize = v1::TerminalViewport {
        rows: snapshot.rows,
        columns: snapshot.columns,
    }
    .try_into()
    .map_err(protocol_error)?;
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
    #[cfg(unix)]
    next_request_id: AtomicU64,
    #[cfg(unix)]
    mutation: AsyncMutex<LocalMutationState>,
}

#[cfg(unix)]
#[derive(Debug)]
struct LocalMutationState {
    lease: Option<OperationLease>,
    next_sequence: u64,
}

impl LocalClient {
    /// Creates a non-spawning client for one effective user's daemon socket.
    #[must_use]
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            #[cfg(unix)]
            next_request_id: AtomicU64::new(1),
            #[cfg(unix)]
            mutation: AsyncMutex::new(LocalMutationState {
                lease: None,
                next_sequence: 1,
            }),
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

    /// Lists live sessions through one strict unary request.
    #[cfg(unix)]
    pub async fn list_sessions(&self) -> Result<Vec<crate::session::SessionSummary>, DaemonError> {
        let frame = self
            .request(
                WireKind::SessionListRequest,
                WireKind::SessionListResponse,
                &v1::SessionListRequest {
                    target: Some(local_target()),
                },
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v1::SessionListResponse = decode_response(&frame)?;
        response
            .sessions
            .into_iter()
            .map(session_from_wire)
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
        let frame = self
            .mutation_request(WireKind::SessionCreateRequest, |operation_id| {
                v1::SessionCreateRequest {
                    operation_id: Some(operation_id.into()),
                    target: Some(local_target()),
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
        let frame = self
            .mutation_request(WireKind::SessionRenameRequest, |operation_id| {
                v1::SessionRenameRequest {
                    operation_id: Some(operation_id.into()),
                    target: Some(local_target()),
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
        let frame = self
            .mutation_request(WireKind::SessionCloseRequest, |operation_id| {
                v1::SessionCloseRequest {
                    operation_id: Some(operation_id.into()),
                    target: Some(local_target()),
                    session_id: Some(session_id.into()),
                }
            })
            .await?;
        mutate_response(frame)
    }

    #[cfg(unix)]
    async fn mutation_request<Message, Build>(
        &self,
        request_kind: WireKind,
        build: Build,
    ) -> Result<DecodedFrame, DaemonError>
    where
        Message: prost::Message,
        Build: FnOnce(OperationId) -> Message,
    {
        // Serializing one logical client's mutation stream keeps lease rotation
        // and poison handling exact; unrelated clients/operation keys still run
        // concurrently in the daemon.
        let mut mutation = self.mutation.lock().await;
        if mutation.lease.is_none() {
            mutation.lease = Some(self.issue_operation_lease().await?);
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
            .request_with_retry(
                request_kind,
                WireKind::SessionMutateResponse,
                &build(operation_id),
                DEFAULT_DEADLINE,
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
    async fn issue_operation_lease(&self) -> Result<OperationLease, DaemonError> {
        let frame = self
            .request_with_retry(
                WireKind::SessionOperationLeaseRequest,
                WireKind::SessionOperationLeaseResponse,
                &v1::SessionOperationLeaseRequest {
                    target: Some(local_target()),
                },
                DEFAULT_DEADLINE,
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
        let bytes = encode_message(request_kind, request_id, deadline_ms, message)
            .map_err(protocol_error)?;
        let absolute_deadline = Instant::now() + deadline;
        let attempts = if retry_ambiguous { 2 } else { 1 };
        let mut last_error = None;
        for _ in 0..attempts {
            match self.request_bytes_once(&bytes, absolute_deadline).await {
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
    pub async fn list_sessions(&self) -> Result<Vec<crate::session::SessionSummary>, DaemonError> {
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
    pub async fn rename_session(
        &self,
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
fn mutate_response(frame: DecodedFrame) -> Result<crate::session::SessionSummary, DaemonError> {
    let response: v1::SessionMutateResponse = decode_response(&frame)?;
    session_from_wire(
        response
            .session
            .ok_or_else(|| malformed("session mutation response omitted session"))?,
    )
}

#[cfg(unix)]
fn session_from_wire(
    summary: v1::SessionSummary,
) -> Result<crate::session::SessionSummary, DaemonError> {
    let session_id = summary
        .session_id
        .ok_or_else(|| malformed("session summary omitted session_id"))?
        .try_into()
        .map_err(protocol_error)?;
    let name = SessionName::new(summary.name)
        .map_err(|error| DaemonError::new(DomainErrorKind::MalformedFrame, error.to_string()))?;
    let viewport = summary
        .viewport
        .ok_or_else(|| malformed("session summary omitted viewport"))?
        .try_into()
        .map_err(protocol_error)?;
    Ok(crate::session::SessionSummary {
        session_id,
        name,
        revision: Revision::new(summary.revision),
        has_controller: summary.has_controller,
        working_directory: PathBuf::from(summary.working_directory),
        viewport,
    })
}

#[cfg(unix)]
fn local_target() -> v1::TargetSelector {
    v1::TargetSelector {
        target: Some(v1::target_selector::Target::Local(true)),
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
    use super::*;

    #[test]
    fn local_attachment_views_are_fresh_fixed_width_ids() {
        let first = local_view_id();
        let second = local_view_id();
        assert_ne!(first, second);
        assert_eq!(first.to_bytes().len(), AttachmentId::LENGTH);
    }
}
