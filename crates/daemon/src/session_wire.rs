//! Shared Session framing and transport-independent service dispatch.
//!
//! The local adapter invokes this module only after its same-UID credential
//! gate. The remote adapter supplies only the TLS-authenticated device and the
//! receiver-owned authorization generation; both paths terminate in the same
//! SessionService and framing owner.

#[cfg(unix)]
use std::collections::VecDeque;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
#[cfg(unix)]
use tokio::sync::{mpsc, oneshot};
#[cfg(unix)]
use zeroize::Zeroizing;
#[cfg(unix)]
use zterm_core::terminal::{
    MAX_HISTORY_PAGE_ROWS, TerminalHistoryCursor, TerminalHistoryDirection,
};
#[cfg(unix)]
use zterm_core::{
    AttachmentId, AttachmentPrincipal, AuthGeneration, DeviceId, DomainErrorKind, OperationId,
    ResourceLimits, ResumeViewId, Revision, SessionId, SessionName, SessionSelector,
};
#[cfg(unix)]
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v1};

#[cfg(unix)]
use crate::authorization::AuthorizationRegistry;
#[cfg(unix)]
use crate::connection_broker::{
    InboundAuthenticatedStream, RemoteServiceHandler, RemoteServiceHandlerFuture,
};
#[cfg(unix)]
use crate::error::DaemonError;
#[cfg(unix)]
use crate::service::{ServiceReply, protocol_error};
#[cfg(unix)]
use crate::session::{
    AttachmentLifecycle, AttachmentUpdate, PreparedAttachment, RemoteAttachmentRequest,
    RemoteResumeRequest, SessionAttachment, SessionService, SessionSummary,
};

#[cfg(unix)]
const ATTACHMENT_OUTBOUND_CAPACITY: usize = 8;

#[cfg(unix)]
const DEFAULT_SESSION_WIRE_DEADLINE: Duration = Duration::from_secs(5);

/// One decoded first frame plus the single decoder's retained leftovers.
#[cfg(unix)]
pub(crate) struct FirstFrame {
    pub(crate) frame: DecodedFrame,
    pub(crate) decoder: FrameDecoder,
    pub(crate) queued: VecDeque<DecodedFrame>,
}

/// Deadlines consumed by the shared Session stream owner.
#[cfg(unix)]
#[derive(Clone, Copy)]
pub(crate) struct SessionWireLimits {
    default_request_deadline: Duration,
    maximum_request_deadline: Duration,
    operation_timeout: Duration,
}

#[cfg(unix)]
impl SessionWireLimits {
    pub(crate) const fn new(
        default_request_deadline: Duration,
        maximum_request_deadline: Duration,
        operation_timeout: Duration,
    ) -> Self {
        Self {
            default_request_deadline,
            maximum_request_deadline,
            operation_timeout,
        }
    }

    pub(crate) fn request_deadline(self, requested_ms: u32) -> Duration {
        if requested_ms == 0 {
            self.default_request_deadline
        } else {
            Duration::from_millis(u64::from(requested_ms)).min(self.maximum_request_deadline)
        }
    }

    pub(crate) const fn operation_timeout(self) -> Duration {
        self.operation_timeout
    }

    fn request_deadline_from(self, started: Instant, requested_ms: u32) -> Instant {
        started
            .checked_add(self.request_deadline(requested_ms))
            .unwrap_or(started)
    }
}

#[cfg(unix)]
impl Default for SessionWireLimits {
    fn default() -> Self {
        Self::new(
            DEFAULT_SESSION_WIRE_DEADLINE,
            Duration::from_secs(u64::from(
                ResourceLimits::default().max_local_deadline_seconds,
            )),
            DEFAULT_SESSION_WIRE_DEADLINE,
        )
    }
}

/// Validated trust and routing context for one Session wire stream.
#[cfg(unix)]
#[derive(Clone)]
pub(crate) enum SessionRequestContext {
    LocalSameUid {
        local_view_id: AttachmentId,
    },
    RemoteAuthenticated {
        own_device_id: DeviceId,
        remote_device_id: DeviceId,
        accepted_generation: AuthGeneration,
        authorization: AuthorizationRegistry,
        #[cfg(test)]
        commit_first_poll_observer: Option<mpsc::UnboundedSender<(DeviceId, bool)>>,
    },
}

#[cfg(unix)]
impl SessionRequestContext {
    fn local(local_view_id: AttachmentId) -> Self {
        Self::LocalSameUid { local_view_id }
    }

    fn remote(
        own_device_id: DeviceId,
        remote_device_id: DeviceId,
        accepted_generation: AuthGeneration,
        authorization: AuthorizationRegistry,
    ) -> Result<Self, DaemonError> {
        if own_device_id == remote_device_id || accepted_generation == AuthGeneration::ZERO {
            return Err(DaemonError::new(
                DomainErrorKind::Unauthorized,
                "remote Session context is not an admitted device generation",
            ));
        }
        Ok(Self::RemoteAuthenticated {
            own_device_id,
            remote_device_id,
            accepted_generation,
            authorization,
            #[cfg(test)]
            commit_first_poll_observer: None,
        })
    }

    #[cfg(test)]
    fn with_commit_first_poll_observer(
        mut self,
        observer: mpsc::UnboundedSender<(DeviceId, bool)>,
    ) -> Self {
        if let Self::RemoteAuthenticated {
            commit_first_poll_observer,
            ..
        } = &mut self
        {
            *commit_first_poll_observer = Some(observer);
        }
        self
    }

    #[cfg(test)]
    fn commit_first_poll_observer(&self) -> Option<&mpsc::UnboundedSender<(DeviceId, bool)>> {
        match self {
            Self::RemoteAuthenticated {
                commit_first_poll_observer,
                ..
            } => commit_first_poll_observer.as_ref(),
            Self::LocalSameUid { .. } => None,
        }
    }

    fn principal(&self, sessions: &SessionService) -> AttachmentPrincipal {
        match self {
            Self::LocalSameUid { local_view_id } => sessions.local_principal(*local_view_id),
            Self::RemoteAuthenticated {
                remote_device_id,
                accepted_generation,
                ..
            } => AttachmentPrincipal::RemoteEndpoint {
                device_id: *remote_device_id,
                auth_generation: accepted_generation.get(),
            },
        }
    }

    fn is_remote(&self) -> bool {
        !matches!(self, Self::LocalSameUid { .. })
    }

    fn require_target(&self, target: Option<v1::TargetSelector>) -> Result<(), DaemonError> {
        self.require_target_with_local_detail(
            target,
            "local session request requires target.local=true",
        )
    }

    fn require_terminal_target(
        &self,
        target: Option<v1::TargetSelector>,
    ) -> Result<(), DaemonError> {
        self.require_target_with_local_detail(
            target,
            "local terminal stream requires target.local=true",
        )
    }

    fn require_target_with_local_detail(
        &self,
        target: Option<v1::TargetSelector>,
        local_detail: &'static str,
    ) -> Result<(), DaemonError> {
        match (self, target.and_then(|target| target.target)) {
            (Self::LocalSameUid { .. }, Some(v1::target_selector::Target::Local(true))) => Ok(()),
            (Self::LocalSameUid { .. }, _) => Err(malformed(local_detail)),
            (
                Self::RemoteAuthenticated { own_device_id, .. },
                Some(v1::target_selector::Target::Device(device)),
            ) => {
                let target: DeviceId = device.try_into().map_err(protocol_error)?;
                if target == *own_device_id {
                    Ok(())
                } else {
                    Err(remote_target_mismatch())
                }
            }
            (Self::RemoteAuthenticated { .. }, _) => Err(remote_target_mismatch()),
        }
    }

    async fn run_effect<T, F>(
        &self,
        sessions: &SessionService,
        deadline: Instant,
        operation: F,
    ) -> Result<T, DaemonError>
    where
        T: Send + 'static,
        F: FnOnce(SessionService, AttachmentPrincipal) -> Result<T, DaemonError> + Send + 'static,
    {
        let sessions = sessions.clone();
        let principal = self.principal(&sessions);
        #[cfg(test)]
        let commit_first_poll_observer = self.commit_first_poll_observer();
        match self {
            Self::LocalSameUid { .. } => {
                run_blocking_until(deadline, move || operation(sessions, principal)).await
            }
            Self::RemoteAuthenticated {
                remote_device_id,
                accepted_generation,
                authorization,
                ..
            } => {
                #[cfg(test)]
                let acquire = async {
                    if let Some(observer) = commit_first_poll_observer {
                        let acquire = tokio::task::unconstrained(
                            authorization.acquire_commit(*remote_device_id, *accepted_generation),
                        );
                        tokio::pin!(acquire);
                        let mut observer = Some(observer);
                        std::future::poll_fn(|context| {
                            let result = acquire.as_mut().poll(context);
                            if let Some(observer) = observer.take() {
                                let _ = observer.send((*remote_device_id, result.is_pending()));
                            }
                            result
                        })
                        .await
                    } else {
                        authorization
                            .acquire_commit(*remote_device_id, *accepted_generation)
                            .await
                    }
                };
                #[cfg(not(test))]
                let acquire = authorization.acquire_commit(*remote_device_id, *accepted_generation);
                let commit = timeout_at(
                    deadline,
                    acquire,
                    "remote authorization commit acquisition exceeded its absolute deadline",
                )
                .await?
                .map_err(project_remote_authorization_error)?;
                Ok(timeout_at(
                    deadline,
                    commit.run(move || operation(sessions, principal)),
                    "remote Session effect exceeded its absolute deadline",
                )
                .await??)
            }
        }
    }
}

#[cfg(unix)]
fn project_remote_authorization_error(error: DaemonError) -> DaemonError {
    if matches!(
        error.kind(),
        DomainErrorKind::Unauthorized | DomainErrorKind::AuthorizationRevoked
    ) {
        DaemonError::new(
            DomainErrorKind::Unauthorized,
            "device is not authorized to control this host",
        )
    } else {
        error
    }
}

#[cfg(unix)]
struct AttachmentOutbound {
    bytes: Zeroizing<Vec<u8>>,
    flushed: Option<oneshot::Sender<()>>,
    deadline: Instant,
}

#[cfg(unix)]
impl AttachmentOutbound {
    fn queued(bytes: Vec<u8>, deadline: Instant) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
            flushed: None,
            deadline,
        }
    }
}

/// Production callback which connects authenticated broker streams to the one
/// transport-independent Session wire owner.
#[cfg(unix)]
#[derive(Clone)]
pub(crate) struct RemoteSessionServiceHandler {
    server: SessionWireServer,
    own_device_id: DeviceId,
    authorization: AuthorizationRegistry,
    limits: SessionWireLimits,
}

#[cfg(unix)]
impl RemoteSessionServiceHandler {
    pub(crate) fn new(
        sessions: SessionService,
        own_device_id: DeviceId,
        authorization: AuthorizationRegistry,
    ) -> Self {
        Self {
            server: SessionWireServer::new(sessions),
            own_device_id,
            authorization,
            limits: SessionWireLimits::default(),
        }
    }
}

#[cfg(unix)]
impl RemoteServiceHandler for RemoteSessionServiceHandler {
    fn handle_service_stream(
        &self,
        stream: InboundAuthenticatedStream,
        first_frame_deadline: Instant,
    ) -> RemoteServiceHandlerFuture {
        let server = self.server.clone();
        let authorization = self.authorization.clone();
        let own_device_id = self.own_device_id;
        let limits = self.limits;
        Box::pin(async move {
            let remote_device_id = stream.remote_device_id();
            let accepted_generation = stream.accepted_generation();
            let context = SessionRequestContext::remote(
                own_device_id,
                remote_device_id,
                accepted_generation,
                authorization,
            )?;
            let (send, recv) = stream.into_parts();
            let stream = tokio::io::join(recv, send);
            server
                .handle_remote_stream(stream, context, limits, first_frame_deadline)
                .await
        })
    }
}

/// Reads exactly through the first complete frame while retaining decoder
/// state and any additional frames received by the same bounded read.
#[cfg(unix)]
pub(crate) async fn read_first<Reader>(reader: &mut Reader) -> Result<FirstFrame, DaemonError>
where
    Reader: AsyncRead + Unpin,
{
    let mut decoder = FrameDecoder::new();
    let mut buffer = Zeroizing::new([0_u8; 16 * 1024]);
    loop {
        let read = reader
            .read(&mut *buffer)
            .await
            .map_err(|error| daemon_io("read Session request", error))?;
        if read == 0 {
            decoder.finish().map_err(protocol_error)?;
            return Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                "client closed before sending a Session request",
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

/// Requires EOF after the one decoded unary frame, including bytes that arrive
/// in a later read.
#[cfg(unix)]
pub(crate) async fn finish_unary<Reader>(
    reader: &mut Reader,
    mut first: FirstFrame,
) -> Result<DecodedFrame, DaemonError>
where
    Reader: AsyncRead + Unpin,
{
    if !first.queued.is_empty() {
        return Err(malformed(
            "one unary connection may contain only one request",
        ));
    }
    let mut buffer = Zeroizing::new([0_u8; 16 * 1024]);
    loop {
        let read = reader
            .read(&mut *buffer)
            .await
            .map_err(|error| daemon_io("finish Session unary request", error))?;
        if read == 0 {
            first.decoder.finish().map_err(protocol_error)?;
            return Ok(first.frame);
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

/// The single wire adapter around the daemon's transport-independent SessionService.
#[cfg(unix)]
#[derive(Clone)]
pub(crate) struct SessionWireServer {
    sessions: SessionService,
}

#[cfg(unix)]
impl SessionWireServer {
    pub(crate) const fn new(sessions: SessionService) -> Self {
        Self { sessions }
    }

    pub(crate) const fn handles_unary(kind: WireKind) -> bool {
        matches!(
            kind,
            WireKind::SessionListRequest
                | WireKind::SessionOperationLeaseRequest
                | WireKind::SessionCreateRequest
                | WireKind::SessionRenameRequest
                | WireKind::SessionCloseRequest
                | WireKind::SessionTakeoverRequest
        )
    }

    /// Dispatches one strict, already-framed same-UID Session request without
    /// blocking the local IPC runtime thread.
    pub(crate) async fn dispatch_local_unary_until(
        &self,
        frame: DecodedFrame,
        deadline: Instant,
    ) -> ServiceReply {
        let context = SessionRequestContext::local(local_request_view_id(frame.request_id));
        self.dispatch_unary_until(&frame, context, deadline).await
    }

    /// Owns first-frame classification for one authenticated remote service
    /// stream. The broker deliberately passes untouched stream halves here.
    pub(crate) async fn handle_remote_stream<Stream>(
        &self,
        mut stream: Stream,
        context: SessionRequestContext,
        limits: SessionWireLimits,
        first_frame_deadline: Instant,
    ) -> Result<(), DaemonError>
    where
        Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let started = Instant::now();
        let first = match timeout_at(
            first_frame_deadline,
            read_first(&mut stream),
            "remote service first frame exceeded its absolute deadline",
        )
        .await
        {
            Ok(Ok(first)) => first,
            Ok(Err(error)) | Err(error) => {
                write_error_best_effort(&mut stream, 0, &error, first_frame_deadline).await;
                return Err(error);
            }
        };
        let request_id = first.frame.request_id;
        let request_deadline = limits.request_deadline_from(started, first.frame.deadline_ms);
        if first.frame.kind == WireKind::TerminalAttachRequest {
            return self
                .handle_attachment(stream, first, context, limits, request_deadline)
                .await;
        }
        if !Self::handles_unary(first.frame.kind) {
            let error = DaemonError::new(
                DomainErrorKind::ServiceNotImplemented,
                format!(
                    "wire service {:?} is not implemented by this Session server",
                    first.frame.kind
                ),
            );
            write_error_best_effort(&mut stream, request_id, &error, request_deadline).await;
            return Err(error);
        }
        let frame = match timeout_at(
            request_deadline,
            finish_unary(&mut stream, first),
            "remote unary request exceeded its absolute deadline",
        )
        .await
        {
            Ok(Ok(frame)) => frame,
            Ok(Err(error)) | Err(error) => {
                write_error_best_effort(&mut stream, request_id, &error, request_deadline).await;
                return Err(error);
            }
        };
        let reply = self
            .dispatch_unary_until(&frame, context, request_deadline)
            .await;
        timeout_at(
            request_deadline,
            stream.write_all(&reply.bytes),
            "remote unary response exceeded its absolute deadline",
        )
        .await?
        .map_err(|error| daemon_io("write remote unary response", error))?;
        timeout_at(
            request_deadline,
            stream.shutdown(),
            "remote unary finish exceeded its absolute deadline",
        )
        .await?
        .map_err(|error| daemon_io("finish remote unary response", error))
    }

    /// Runs one same-UID terminal attachment over generic bounded async I/O.
    pub(crate) async fn handle_local_attachment<Stream>(
        &self,
        stream: Stream,
        first: FirstFrame,
        local_view_id: AttachmentId,
        limits: SessionWireLimits,
        deadline: Instant,
    ) -> Result<(), DaemonError>
    where
        Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.handle_attachment(
            stream,
            first,
            SessionRequestContext::local(local_view_id),
            limits,
            deadline,
        )
        .await
    }

    async fn handle_attachment<Stream>(
        &self,
        mut stream: Stream,
        first: FirstFrame,
        context: SessionRequestContext,
        limits: SessionWireLimits,
        deadline: Instant,
    ) -> Result<(), DaemonError>
    where
        Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let prepared = match self
            .prepare_attachment(&context, &first.frame, deadline)
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                write_error_best_effort(&mut stream, first.frame.request_id, &error, deadline)
                    .await;
                return Err(error);
            }
        };
        let attachment = prepared.attachment;
        let initial = if let Some(delta) = prepared.initial_delta {
            let message = zterm_proto::terminal_delta_message(attachment.attachment_id(), delta);
            encode_message(WireKind::TerminalDelta, first.frame.request_id, 0, &message)
                .map_err(protocol_error)?
        } else {
            encode_snapshot(
                first.frame.request_id,
                attachment.session_id(),
                attachment.attachment_id(),
                prepared.snapshot,
            )?
        };
        write_attachment_bytes_until(
            &mut stream,
            &initial,
            deadline,
            "initial terminal update exceeded its absolute deadline",
            "write initial terminal update",
        )
        .await?;
        let (reader, writer) = tokio::io::split(stream);
        let (outbound_sender, outbound_receiver) =
            mpsc::channel::<AttachmentOutbound>(ATTACHMENT_OUTBOUND_CAPACITY);
        let reader_attachment = Arc::clone(&attachment);
        let reader_server = self.clone();
        let reader_context = context.clone();
        let mut reader_task = tokio::spawn(async move {
            attachment_reader(
                reader,
                first.decoder,
                first.queued,
                AttachmentReaderContext {
                    server: reader_server,
                    attachment: reader_attachment,
                    request_context: reader_context,
                    outbound: outbound_sender,
                    limits,
                },
            )
            .await
        });
        let writer_attachment = Arc::clone(&attachment);
        let writer_server = self.clone();
        let writer_context = context.clone();
        let mut writer_task = tokio::spawn(async move {
            attachment_writer(
                writer,
                writer_server,
                writer_attachment,
                writer_context,
                outbound_receiver,
                limits.operation_timeout,
            )
            .await
        });

        let result = tokio::select! {
            biased;
            result = &mut reader_task => {
                writer_task.abort();
                flatten_attachment_task(result)
            }
            result = &mut writer_task => {
                let writer_result = flatten_attachment_task(result);
                if writer_result.as_ref().is_err_and(|error| {
                    error.kind() == DomainErrorKind::DaemonStopped
                }) {
                    // A peer can send an explicit detach and close its read
                    // half while a revision-only terminal update is already
                    // writable. Give the reader its bounded opportunity to
                    // classify the queued detach/EOF before treating the write
                    // failure as authoritative transport loss.
                    match tokio::time::timeout(limits.operation_timeout, &mut reader_task).await {
                        Ok(result) => flatten_attachment_task(result),
                        Err(_) => {
                            reader_task.abort();
                            writer_result
                        }
                    }
                } else {
                    reader_task.abort();
                    writer_result
                }
            }
        };
        let transport_loss = should_move_remote_resume_checkpoint(context.is_remote(), &result);
        if transport_loss {
            let attachment_worker = Arc::clone(&attachment);
            let save_deadline = Instant::now() + limits.operation_timeout;
            if context
                .run_effect(
                    &self.sessions,
                    save_deadline,
                    move |_sessions, _principal| {
                        attachment_worker.detach_for_remote_resume_until(save_deadline)
                    },
                )
                .await
                .is_err()
            {
                attachment.detach();
            }
        } else {
            attachment.detach();
        }
        result.map(|_| ())
    }

    async fn prepare_attachment(
        &self,
        context: &SessionRequestContext,
        frame: &DecodedFrame,
        deadline: Instant,
    ) -> Result<PreparedAttachment, DaemonError> {
        let request: v1::TerminalAttachRequest = frame
            .decode_message(WireKind::TerminalAttachRequest)
            .map_err(protocol_error)?;
        context.require_terminal_target(request.target.clone())?;
        let (selector, create_main) = terminal_selector(&request)?;
        let viewport = request
            .viewport
            .map(TryInto::try_into)
            .transpose()
            .map_err(protocol_error)?;
        let resume = terminal_resume_request(context, &request)?;
        context
            .run_effect(&self.sessions, deadline, move |sessions, principal| {
                if let Some(resume) = resume {
                    sessions.prepare_remote_attach_until(
                        principal,
                        RemoteAttachmentRequest {
                            selector,
                            create_main,
                            takeover: request.takeover,
                            initial_viewport: viewport,
                            resume,
                        },
                        deadline,
                    )
                } else {
                    sessions.prepare_attach_until(
                        principal,
                        selector,
                        create_main,
                        request.takeover,
                        viewport,
                        deadline,
                    )
                }
            })
            .await
    }

    async fn dispatch_unary_until(
        &self,
        frame: &DecodedFrame,
        context: SessionRequestContext,
        deadline: Instant,
    ) -> ServiceReply {
        let request_id = frame.request_id;
        let result: Result<ServiceReply, DaemonError> = async {
            match frame.kind {
                WireKind::SessionListRequest => {
                    let request: v1::SessionListRequest = decode_request(frame)?;
                    context.require_target(request.target)?;
                    let sessions = context
                        .run_effect(&self.sessions, deadline, |sessions, _principal| {
                            sessions.list()
                        })
                        .await?
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
                    let request: v1::SessionOperationLeaseRequest = decode_request(frame)?;
                    context.require_target(request.target)?;
                    let lease = context
                        .run_effect(&self.sessions, deadline, |sessions, principal| {
                            sessions.issue_operation_lease(principal)
                        })
                        .await?;
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
                    let request: v1::SessionCreateRequest = decode_request(frame)?;
                    context.require_target(request.target)?;
                    let operation_id = required_operation_id(request.operation_id)?;
                    let name = session_name(&request.name)?;
                    let working_directory = (!request.working_directory.is_empty())
                        .then(|| PathBuf::from(request.working_directory));
                    let viewport = request
                        .viewport
                        .map(TryInto::try_into)
                        .transpose()
                        .map_err(protocol_error)?;
                    let summary = context
                        .run_effect(&self.sessions, deadline, move |sessions, principal| {
                            sessions.create_until(
                                principal,
                                operation_id,
                                name,
                                working_directory,
                                viewport,
                                deadline,
                            )
                        })
                        .await?;
                    mutate_reply(request_id, summary)
                }
                WireKind::SessionRenameRequest => {
                    let request: v1::SessionRenameRequest = decode_request(frame)?;
                    context.require_target(request.target)?;
                    let operation_id = required_operation_id(request.operation_id)?;
                    let session_id = required_session_id(request.session_id)?;
                    let name = session_name(&request.name)?;
                    let summary = context
                        .run_effect(&self.sessions, deadline, move |sessions, principal| {
                            sessions.rename_until(
                                principal,
                                operation_id,
                                session_id,
                                name,
                                deadline,
                            )
                        })
                        .await?;
                    mutate_reply(request_id, summary)
                }
                WireKind::SessionCloseRequest => {
                    let request: v1::SessionCloseRequest = decode_request(frame)?;
                    context.require_target(request.target)?;
                    let operation_id = required_operation_id(request.operation_id)?;
                    let session_id = required_session_id(request.session_id)?;
                    let summary = context
                        .run_effect(&self.sessions, deadline, move |sessions, principal| {
                            sessions.close_until(principal, operation_id, session_id, deadline)
                        })
                        .await?;
                    mutate_reply(request_id, summary)
                }
                WireKind::SessionTakeoverRequest => {
                    let request: v1::SessionTakeoverRequest = decode_request(frame)?;
                    context.require_target(request.target)?;
                    let attachment_id = request
                        .attachment_id
                        .ok_or_else(|| malformed("takeover omitted attachment_id"))?
                        .try_into()
                        .map_err(protocol_error)?;
                    let operation_id = required_operation_id(request.operation_id)?;
                    let session_id = required_session_id(request.session_id)?;
                    let summary = context
                        .run_effect(&self.sessions, deadline, move |sessions, principal| {
                            sessions.takeover_by_id_until(
                                principal,
                                operation_id,
                                session_id,
                                attachment_id,
                                deadline,
                            )
                        })
                        .await?;
                    mutate_reply(request_id, summary)
                }
                _ => Err(DaemonError::new(
                    DomainErrorKind::ServiceNotImplemented,
                    format!(
                        "wire service {:?} is not implemented by this Session server",
                        frame.kind
                    ),
                )),
            }
        }
        .await;
        match result {
            Ok(reply) => reply,
            Err(error) => ServiceReply::error(request_id, &error),
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttachmentTaskEnd {
    Explicit,
    TransportEof,
    Terminal,
}

#[cfg(unix)]
fn should_move_remote_resume_checkpoint(
    remote_authenticated: bool,
    result: &Result<AttachmentTaskEnd, DaemonError>,
) -> bool {
    remote_authenticated && matches!(result, Ok(AttachmentTaskEnd::TransportEof))
}

#[cfg(unix)]
fn flatten_attachment_task(
    result: Result<Result<AttachmentTaskEnd, DaemonError>, tokio::task::JoinError>,
) -> Result<AttachmentTaskEnd, DaemonError> {
    result.map_err(|error| {
        DaemonError::new(
            DomainErrorKind::Cancelled,
            format!("Session attachment task ended unexpectedly: {error}"),
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
async fn timeout_at<F>(
    deadline: Instant,
    future: F,
    detail: &'static str,
) -> Result<F::Output, DaemonError>
where
    F: std::future::Future,
{
    if Instant::now() >= deadline {
        return Err(DaemonError::new(DomainErrorKind::DeadlineExceeded, detail));
    }
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future)
        .await
        .map_err(|_| DaemonError::new(DomainErrorKind::DeadlineExceeded, detail))
}

#[cfg(unix)]
async fn write_error_best_effort<Writer>(
    writer: &mut Writer,
    request_id: u64,
    error: &DaemonError,
    deadline: Instant,
) where
    Writer: AsyncWrite + Unpin,
{
    let bytes = ServiceReply::error(request_id, error).bytes;
    let write = async {
        writer.write_all(&bytes).await?;
        writer.shutdown().await
    };
    let _ = timeout_at(deadline, write, "Session error response deadline elapsed").await;
}

#[cfg(unix)]
async fn write_attachment_bytes_until<Writer>(
    writer: &mut Writer,
    bytes: &[u8],
    deadline: Instant,
    deadline_detail: &'static str,
    operation: &'static str,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    timeout_at(
        deadline,
        async {
            writer.write_all(bytes).await?;
            writer.flush().await
        },
        deadline_detail,
    )
    .await?
    .map_err(|error| daemon_io(operation, error))
}

#[cfg(unix)]
async fn send_attachment_outbound_until(
    outbound: &mpsc::Sender<AttachmentOutbound>,
    message: AttachmentOutbound,
    deadline: Instant,
) -> Result<(), DaemonError> {
    timeout_at(
        deadline,
        outbound.send(message),
        "terminal response queue exceeded its absolute deadline",
    )
    .await?
    .map_err(|_| attachment_cancelled())
}

#[cfg(unix)]
struct AttachmentReaderContext {
    server: SessionWireServer,
    attachment: Arc<SessionAttachment>,
    request_context: SessionRequestContext,
    outbound: mpsc::Sender<AttachmentOutbound>,
    limits: SessionWireLimits,
}

#[cfg(unix)]
async fn attachment_reader<Reader>(
    mut reader: Reader,
    mut decoder: FrameDecoder,
    mut queued: VecDeque<DecodedFrame>,
    context: AttachmentReaderContext,
) -> Result<AttachmentTaskEnd, DaemonError>
where
    Reader: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        if let Some(frame) = queued.pop_front() {
            match process_attachment_frame(
                frame.clone(),
                &context.server,
                &context.attachment,
                &context.request_context,
                &context.outbound,
                context.limits,
            )
            .await
            {
                Ok(false) => continue,
                Ok(true) => return Ok(AttachmentTaskEnd::Explicit),
                Err(error) => {
                    flush_attachment_error(
                        &context.outbound,
                        frame.request_id,
                        &error,
                        context.limits.operation_timeout,
                    )
                    .await;
                    return Err(error);
                }
            }
        }

        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| daemon_io("read terminal stream", error))?;
        if read == 0 {
            if let Err(error) = decoder.finish() {
                let error = protocol_error(error);
                flush_attachment_error(
                    &context.outbound,
                    0,
                    &error,
                    context.limits.operation_timeout,
                )
                .await;
                return Err(error);
            }
            return Ok(AttachmentTaskEnd::TransportEof);
        }
        match decoder.feed(&buffer[..read]) {
            Ok(frames) => queued.extend(frames),
            Err(error) => {
                let error = protocol_error(error);
                flush_attachment_error(
                    &context.outbound,
                    0,
                    &error,
                    context.limits.operation_timeout,
                )
                .await;
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
    operation_timeout: Duration,
) {
    let (flushed, wait_for_flush) = oneshot::channel();
    let deadline = Instant::now() + operation_timeout;
    let flush = async {
        send_attachment_outbound_until(
            outbound,
            AttachmentOutbound {
                bytes: ServiceReply::error(request_id, error).bytes,
                flushed: Some(flushed),
                deadline,
            },
            deadline,
        )
        .await
        .map_err(|_| ())?;
        wait_for_flush.await.map_err(|_| ())
    };
    let _ = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), flush).await;
}

#[cfg(unix)]
async fn process_attachment_frame(
    frame: DecodedFrame,
    server: &SessionWireServer,
    attachment: &Arc<SessionAttachment>,
    request_context: &SessionRequestContext,
    outbound: &mpsc::Sender<AttachmentOutbound>,
    limits: SessionWireLimits,
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
            let snapshot = request_context
                .run_effect(&server.sessions, deadline, move |_sessions, _principal| {
                    attachment_worker.snapshot_applied_until(revision, deadline)
                })
                .await?;
            if let Some(snapshot) = snapshot {
                send_resync(frame.request_id, attachment, snapshot, outbound, deadline).await?;
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
            let snapshot = request_context
                .run_effect(&server.sessions, deadline, move |_sessions, _principal| {
                    attachment_worker.sync_latest_until(known_revision, deadline)
                })
                .await?;
            send_resync(frame.request_id, attachment, snapshot, outbound, deadline).await?;
            Ok(false)
        }
        WireKind::TerminalHistoryRequest => {
            let request: v1::TerminalHistoryRequest = frame
                .decode_message(WireKind::TerminalHistoryRequest)
                .map_err(protocol_error)?;
            require_attachment_id(request.attachment_id, attachment)?;
            let direction = terminal_history_direction(request.direction)?;
            let cursor = request.cursor.map(terminal_history_cursor);
            let maximum_rows = usize::try_from(request.maximum_rows)
                .map_err(|_| malformed("terminal history page bound is not representable"))?;
            if maximum_rows == 0 || maximum_rows > MAX_HISTORY_PAGE_ROWS {
                return Err(malformed(
                    "terminal history page bound is outside the allowed range",
                ));
            }
            let attachment_worker = Arc::clone(attachment);
            let result = request_context
                .run_effect(&server.sessions, deadline, move |_sessions, _principal| {
                    attachment_worker.history_page_until(direction, cursor, maximum_rows, deadline)
                })
                .await?;
            let message =
                zterm_proto::terminal_history_page_message(attachment.attachment_id(), result);
            send_attachment_outbound_until(
                outbound,
                AttachmentOutbound::queued(
                    encode_message(WireKind::TerminalHistoryPage, frame.request_id, 0, &message)
                        .map_err(protocol_error)?,
                    deadline,
                ),
                deadline,
            )
            .await?;
            Ok(false)
        }
        WireKind::TerminalInput => {
            let request: v1::TerminalInput = frame
                .decode_message(WireKind::TerminalInput)
                .map_err(protocol_error)?;
            require_attachment_id(request.attachment_id, attachment)?;
            let attachment_worker = Arc::clone(attachment);
            request_context
                .run_effect(&server.sessions, deadline, move |_sessions, _principal| {
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
            request_context
                .run_effect(&server.sessions, deadline, move |_sessions, _principal| {
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
            let attachment_worker = Arc::clone(attachment);
            request_context
                .run_effect(&server.sessions, deadline, move |_sessions, _principal| {
                    attachment_worker.detach();
                    Ok(())
                })
                .await?;
            Ok(true)
        }
        WireKind::SessionOperationLeaseRequest => {
            let request: v1::SessionOperationLeaseRequest = frame
                .decode_message(WireKind::SessionOperationLeaseRequest)
                .map_err(protocol_error)?;
            request_context.require_terminal_target(request.target)?;
            let lease = request_context
                .run_effect(&server.sessions, deadline, |sessions, principal| {
                    sessions.issue_operation_lease(principal)
                })
                .await?;
            send_attachment_outbound_until(
                outbound,
                AttachmentOutbound::queued(
                    encode_message(
                        WireKind::SessionOperationLeaseResponse,
                        frame.request_id,
                        0,
                        &v1::SessionOperationLeaseResponse {
                            lease: Some(lease.into()),
                        },
                    )
                    .map_err(protocol_error)?,
                    deadline,
                ),
                deadline,
            )
            .await?;
            Ok(false)
        }
        WireKind::SessionTakeoverRequest => {
            let request: v1::SessionTakeoverRequest = frame
                .decode_message(WireKind::SessionTakeoverRequest)
                .map_err(protocol_error)?;
            request_context.require_terminal_target(request.target.clone())?;
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
            let attachment_worker = Arc::clone(attachment);
            let summary = request_context
                .run_effect(&server.sessions, deadline, move |sessions, principal| {
                    sessions.takeover_until(principal, operation_id, &attachment_worker, deadline)
                })
                .await?;
            let message = v1::SessionMutateResponse {
                session: Some(session_summary_proto(summary)),
            };
            send_attachment_outbound_until(
                outbound,
                AttachmentOutbound::queued(
                    encode_message(
                        WireKind::SessionMutateResponse,
                        frame.request_id,
                        0,
                        &message,
                    )
                    .map_err(protocol_error)?,
                    deadline,
                ),
                deadline,
            )
            .await?;
            Ok(false)
        }
        _ => Err(malformed(format!(
            "wire kind {:?} is invalid on a terminal attachment",
            frame.kind
        ))),
    }
}

#[cfg(unix)]
async fn attachment_writer<Writer>(
    mut writer: Writer,
    server: SessionWireServer,
    attachment: Arc<SessionAttachment>,
    request_context: SessionRequestContext,
    mut outbound: mpsc::Receiver<AttachmentOutbound>,
    operation_timeout: Duration,
) -> Result<AttachmentTaskEnd, DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    let mut revisions = attachment.revision_watch()?;
    let mut lifecycle = attachment.lifecycle_watch()?;
    let mut revisions_open = true;
    let initial_lifecycle = lifecycle.borrow().clone();
    if write_lifecycle_event(
        &mut writer,
        &server,
        &attachment,
        &request_context,
        initial_lifecycle,
        Instant::now() + operation_timeout,
    )
    .await?
    {
        return Ok(AttachmentTaskEnd::Terminal);
    }
    loop {
        tokio::select! {
            biased;
            message = outbound.recv() => {
                let Some(message) = message else {
                    return Ok(AttachmentTaskEnd::Terminal);
                };
                write_attachment_bytes_until(
                    &mut writer,
                    &message.bytes,
                    message.deadline,
                    "terminal response exceeded its absolute deadline",
                    "write terminal response",
                ).await?;
                if let Some(flushed) = message.flushed {
                    let _ = flushed.send(());
                }
            }
            changed = lifecycle.changed() => {
                changed.map_err(|_| attachment_cancelled())?;
                let event = lifecycle.borrow_and_update().clone();
                if write_lifecycle_event(
                    &mut writer,
                    &server,
                    &attachment,
                    &request_context,
                    event,
                    Instant::now() + operation_timeout,
                ).await? {
                    return Ok(AttachmentTaskEnd::Terminal);
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
                    let deadline = Instant::now() + operation_timeout;
                    match attachment_next_update(
                        &server,
                        Arc::clone(&attachment),
                        &request_context,
                        deadline,
                    ).await {
                        Ok(Some(update)) => {
                            write_terminal_update(
                                &mut writer,
                                &attachment,
                                update,
                                deadline,
                            ).await?;
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
async fn write_lifecycle_event<Writer>(
    writer: &mut Writer,
    server: &SessionWireServer,
    attachment: &Arc<SessionAttachment>,
    request_context: &SessionRequestContext,
    event: AttachmentLifecycle,
    deadline: Instant,
) -> Result<bool, DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    match event {
        AttachmentLifecycle::AwaitingSnapshot { .. } => Ok(false),
        AttachmentLifecycle::Active { .. } | AttachmentLifecycle::PreparedTakeover => {
            if let Some(update) =
                attachment_next_update(server, Arc::clone(attachment), request_context, deadline)
                    .await?
            {
                write_terminal_update(writer, attachment, update, deadline).await?;
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
            write_attachment_bytes_until(
                writer,
                &bytes,
                deadline,
                "lease-lost event exceeded its absolute deadline",
                "write lease-lost event",
            )
            .await?;
            Ok(true)
        }
        AttachmentLifecycle::SessionEnded(reason) => {
            let final_attachment = Arc::clone(attachment);
            if let Ok(Some(update)) = request_context
                .run_effect(&server.sessions, deadline, move |_sessions, _principal| {
                    final_attachment.final_update_until(deadline)
                })
                .await
            {
                write_terminal_update(writer, attachment, update, deadline).await?;
            }
            let message = session_ended_message(attachment, reason);
            let bytes = encode_message(WireKind::TerminalSessionEnded, 0, 0, &message)
                .map_err(protocol_error)?;
            write_attachment_bytes_until(
                writer,
                &bytes,
                deadline,
                "session-ended event exceeded its absolute deadline",
                "write session-ended event",
            )
            .await?;
            Ok(true)
        }
    }
}

#[cfg(unix)]
async fn attachment_next_update(
    server: &SessionWireServer,
    attachment: Arc<SessionAttachment>,
    request_context: &SessionRequestContext,
    deadline: Instant,
) -> Result<Option<AttachmentUpdate>, DaemonError> {
    request_context
        .run_effect(&server.sessions, deadline, move |_sessions, _principal| {
            attachment.next_update_until(deadline)
        })
        .await
}

#[cfg(unix)]
async fn write_terminal_update<Writer>(
    writer: &mut Writer,
    attachment: &SessionAttachment,
    update: AttachmentUpdate,
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    let (bytes, operation) = match update {
        AttachmentUpdate::Delta(delta) => {
            let message = zterm_proto::terminal_delta_message(attachment.attachment_id(), delta);
            let bytes =
                encode_message(WireKind::TerminalDelta, 0, 0, &message).map_err(protocol_error)?;
            (bytes, "write terminal delta")
        }
        AttachmentUpdate::Snapshot(snapshot) => {
            let required = v1::TerminalSyncRequired {
                attachment_id: Some(attachment.attachment_id().into()),
                latest_revision: snapshot.revision.get(),
            };
            let mut bytes = encode_message(WireKind::TerminalSyncRequired, 0, 0, &required)
                .map_err(protocol_error)?;
            let snapshot = encode_snapshot(
                0,
                attachment.session_id(),
                attachment.attachment_id(),
                snapshot,
            )?;
            bytes.extend_from_slice(&snapshot);
            (bytes, "write terminal resync update")
        }
    };
    write_attachment_bytes_until(
        writer,
        &bytes,
        deadline,
        "terminal update exceeded its absolute deadline",
        operation,
    )
    .await
}

#[cfg(unix)]
async fn send_resync(
    request_id: u64,
    attachment: &SessionAttachment,
    snapshot: zterm_core::terminal::TerminalSnapshot,
    outbound: &mpsc::Sender<AttachmentOutbound>,
    deadline: Instant,
) -> Result<(), DaemonError> {
    let required = v1::TerminalSyncRequired {
        attachment_id: Some(attachment.attachment_id().into()),
        latest_revision: snapshot.revision.get(),
    };
    let mut bytes = encode_message(WireKind::TerminalSyncRequired, request_id, 0, &required)
        .map_err(protocol_error)?;
    bytes.extend_from_slice(&encode_snapshot(
        request_id,
        attachment.session_id(),
        attachment.attachment_id(),
        snapshot,
    )?);
    send_attachment_outbound_until(
        outbound,
        AttachmentOutbound::queued(bytes, deadline),
        deadline,
    )
    .await
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
fn terminal_resume_request(
    context: &SessionRequestContext,
    request: &v1::TerminalAttachRequest,
) -> Result<Option<RemoteResumeRequest>, DaemonError> {
    if !context.is_remote() {
        if request.resume_view_id.is_some() || request.known_revision.is_some() {
            return Err(malformed(
                "same-UID local attachments cannot submit remote resume state",
            ));
        }
        return Ok(None);
    }

    let Some(view_id) = request.resume_view_id.clone() else {
        if request.known_revision.is_some() {
            return Err(malformed("known_revision requires an exact resume_view_id"));
        }
        return Ok(None);
    };
    let view_id: ResumeViewId = view_id.try_into().map_err(protocol_error)?;
    if view_id.as_bytes().iter().all(|byte| *byte == 0) {
        return Err(malformed("resume_view_id must be random and non-zero"));
    }
    Ok(Some(RemoteResumeRequest {
        view_id,
        known_revision: request.known_revision.map(Revision::new),
    }))
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
fn attachment_cancelled() -> DaemonError {
    DaemonError::new(DomainErrorKind::Cancelled, "terminal attachment closed")
}

#[cfg(unix)]
fn decode_request<Message>(frame: &DecodedFrame) -> Result<Message, DaemonError>
where
    Message: prost::Message + Default,
{
    frame.decode_message(frame.kind).map_err(protocol_error)
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
fn local_request_view_id(request_id: u64) -> AttachmentId {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&request_id.to_le_bytes());
    AttachmentId::from_array(bytes)
}

#[cfg(unix)]
fn terminal_history_direction(value: i32) -> Result<TerminalHistoryDirection, DaemonError> {
    match v1::TerminalHistoryDirection::try_from(value) {
        Ok(v1::TerminalHistoryDirection::Newest) => Ok(TerminalHistoryDirection::Newest),
        Ok(v1::TerminalHistoryDirection::Older) => Ok(TerminalHistoryDirection::Older),
        Ok(v1::TerminalHistoryDirection::Newer) => Ok(TerminalHistoryDirection::Newer),
        Ok(v1::TerminalHistoryDirection::Unspecified) | Err(_) => {
            Err(malformed("terminal history direction is invalid"))
        }
    }
}

#[cfg(unix)]
fn terminal_history_cursor(value: v1::TerminalHistoryCursor) -> TerminalHistoryCursor {
    TerminalHistoryCursor {
        epoch: Revision::new(value.epoch),
        revision: Revision::new(value.revision),
        start_row: value.start_row,
        row_count: value.row_count,
        oldest_row: value.oldest_row,
        newest_row: value.newest_row,
    }
}

#[cfg(unix)]
fn malformed(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

#[cfg(unix)]
fn remote_target_mismatch() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::Unauthorized,
        "remote Session request target does not identify this host",
    )
}

#[cfg(unix)]
fn daemon_io(operation: &str, error: std::io::Error) -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DaemonStopped,
        format!("{operation}: {error}"),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc as std_mpsc;
    use std::sync::{Arc, Mutex, MutexGuard};

    use tokio::io::DuplexStream;
    use tokio::sync::{Notify, mpsc as tokio_mpsc};
    use zterm_core::terminal::TerminalSize;
    use zterm_core::{
        AuthorizationSnapshot, AuthorizationStatus, DeviceDisplayName, DeviceSummary,
        OperationLease,
    };
    use zterm_platform::pty::{ExplicitPtyCommand, PtyHost, PtySize};
    use zterm_platform::user_state::UserPaths;

    use super::*;
    use crate::bootstrap::bootstrap;
    use crate::config::{ValidatedInfrastructure, validate_setup_input};
    use crate::device_directory::DeviceDirectory;
    use crate::service::{
        DaemonService, DeviceLiveObservation, DeviceManagement, RemoteDeviceAccess,
    };
    use crate::store::{
        DeviceAuthorization, StateStore, StoreActor, StoreHandle, default_store_deadline,
    };

    fn device(byte: u8) -> DeviceId {
        DeviceId::from_array([byte; 32])
    }

    fn generation(value: u64) -> AuthGeneration {
        AuthGeneration::new(value).expect("test generation is in range")
    }

    fn authorization_row(
        device_id: DeviceId,
        status: AuthorizationStatus,
        generation: AuthGeneration,
    ) -> DeviceAuthorization {
        DeviceAuthorization {
            device_id,
            display_name: DeviceDisplayName::new("remote fixture").expect("display name"),
            status,
            generation,
            paired_at_unix: 1,
            revoked_at_unix: (status == AuthorizationStatus::Revoked).then_some(2),
            last_seen_at_unix: None,
        }
    }

    fn authorized_registry(remote: DeviceId, accepted: AuthGeneration) -> AuthorizationRegistry {
        let authorization = AuthorizationRegistry::new();
        authorization
            .preload(vec![authorization_row(
                remote,
                AuthorizationStatus::Authorized,
                accepted,
            )])
            .expect("preload remote authorization");
        authorization
    }

    const MATRIX_REMOTE: DeviceId = DeviceId::from_array([0x81; DeviceId::LENGTH]);
    const MATRIX_OTHER: DeviceId = DeviceId::from_array([0x82; DeviceId::LENGTH]);

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct MatrixCloseEvidence {
        device_id: DeviceId,
        durable: AuthorizationSnapshot,
        memory: AuthorizationSnapshot,
        attachments_before_detach: usize,
        sessions_before_detach: usize,
    }

    struct MatrixRemoteAccess {
        store: StoreHandle,
        authorization: AuthorizationRegistry,
        sessions: SessionService,
        close_calls: AtomicUsize,
        evidence: Mutex<Vec<MatrixCloseEvidence>>,
        close_entered: Notify,
        release_first_close: Notify,
    }

    impl MatrixRemoteAccess {
        fn new(
            store: StoreHandle,
            authorization: AuthorizationRegistry,
            sessions: SessionService,
        ) -> Self {
            Self {
                store,
                authorization,
                sessions,
                close_calls: AtomicUsize::new(0),
                evidence: Mutex::new(Vec::new()),
                close_entered: Notify::new(),
                release_first_close: Notify::new(),
            }
        }

        async fn wait_for_close(&self, index: usize) -> MatrixCloseEvidence {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let notified = self.close_entered.notified();
                    if let Some(evidence) = test_lock(&self.evidence).get(index).copied() {
                        return evidence;
                    }
                    notified.await;
                }
            })
            .await
            .expect("real revoke reached the fake close owner")
        }

        fn release_first_close(&self) {
            self.release_first_close.notify_one();
        }

        fn close_count(&self) -> usize {
            self.close_calls.load(Ordering::Acquire)
        }
    }

    impl RemoteDeviceAccess for MatrixRemoteAccess {
        fn observe<'a>(
            &'a self,
            _device_id: DeviceId,
            _deadline: Instant,
        ) -> Pin<Box<dyn Future<Output = Result<DeviceLiveObservation, DaemonError>> + Send + 'a>>
        {
            Box::pin(async { Ok(DeviceLiveObservation::default()) })
        }

        fn close_remote<'a>(
            &'a self,
            device_id: DeviceId,
            deadline: Instant,
        ) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'a>> {
            Box::pin(async move {
                let store = self.store.clone();
                let authorization = self.authorization.clone();
                let sessions = self.sessions.clone();
                let evidence = tokio::task::spawn_blocking(move || {
                    Ok::<_, DaemonError>(MatrixCloseEvidence {
                        device_id,
                        durable: store.authorization_snapshot(device_id, deadline)?,
                        memory: authorization.snapshot(device_id)?,
                        attachments_before_detach: sessions
                            .remote_attachment_count_until(device_id, deadline)?,
                        sessions_before_detach: sessions.list()?.len(),
                    })
                })
                .await
                .map_err(|error| {
                    DaemonError::new(
                        DomainErrorKind::Cancelled,
                        format!("matrix close observer ended unexpectedly: {error}"),
                    )
                })??;
                let call_index = self.close_calls.fetch_add(1, Ordering::AcqRel);
                test_lock(&self.evidence).push(evidence);
                self.close_entered.notify_waiters();
                if call_index == 0 {
                    tokio::time::timeout_at(
                        tokio::time::Instant::from_std(deadline),
                        self.release_first_close.notified(),
                    )
                    .await
                    .map_err(|_| {
                        DaemonError::new(
                            DomainErrorKind::DeadlineExceeded,
                            "matrix close barrier exceeded the revoke deadline",
                        )
                    })?;
                }
                Ok(())
            })
        }
    }

    struct DirectRevokeHarness {
        _temporary: tempfile::TempDir,
        paths: UserPaths,
        actor: StoreActor,
        store: StoreHandle,
        authorization: AuthorizationRegistry,
        service: DaemonService,
        access: Arc<MatrixRemoteAccess>,
        writer_polled: tokio_mpsc::UnboundedReceiver<DeviceId>,
    }

    impl DirectRevokeHarness {
        fn start(sessions: SessionService) -> Self {
            let temporary = tempfile::tempdir().expect("temporary direct-revoke fixture");
            let home = temporary.path().join("home");
            fs::create_dir(&home).expect("direct-revoke fixture home");
            let paths = UserPaths::for_test(
                nix::unistd::Uid::effective().as_raw(),
                home.clone(),
                home.join(".zterm"),
                temporary.path().join("run"),
            );
            let requested =
                validate_setup_input("matrix-host", ValidatedInfrastructure::OfficialN0)
                    .expect("valid direct-revoke setup");
            let setup = bootstrap(&paths, &requested).expect("direct-revoke bootstrap");
            let mut state = StateStore::open(&paths).expect("direct-revoke state store");
            state
                .authorize_device(MATRIX_REMOTE, "matrix remote", 10)
                .expect("authorize matrix remote");
            state
                .authorize_device(MATRIX_OTHER, "matrix other", 11)
                .expect("authorize unaffected remote");
            let rows = state
                .list_authorizations()
                .expect("preload direct-revoke authorizations");
            let actor = StoreActor::start(state).expect("direct-revoke StoreActor");
            let store = actor.handle();
            let authorization = AuthorizationRegistry::new();
            authorization
                .preload(rows)
                .expect("preload direct-revoke registry");
            let directory = DeviceDirectory::new(store.clone());
            let access = Arc::new(MatrixRemoteAccess::new(
                store.clone(),
                authorization.clone(),
                sessions.clone(),
            ));
            let remote_access: Arc<dyn RemoteDeviceAccess> = access.clone();
            let (writer_polled_tx, writer_polled) = tokio_mpsc::unbounded_channel();
            let management = DeviceManagement::new(
                store.clone(),
                directory,
                authorization.clone(),
                remote_access,
            )
            .with_revoke_guard_after_first_poll_for_test(writer_polled_tx);
            let service = DaemonService::with_sessions(setup, 123, sessions)
                .with_device_management(management);
            Self {
                _temporary: temporary,
                paths,
                actor,
                store,
                authorization,
                service,
                access,
                writer_polled,
            }
        }

        async fn wait_for_writer(&mut self, device_id: DeviceId) {
            let observed = tokio::time::timeout(Duration::from_secs(2), self.writer_polled.recv())
                .await
                .expect("revoke writer reached its first fair-lock poll");
            assert_eq!(
                observed,
                Some(device_id),
                "revoke writer notification follows its first real lock poll",
            );
        }

        fn spawn_revoke(
            &self,
            request_id: u64,
            device_id: DeviceId,
        ) -> tokio::task::JoinHandle<DecodedFrame> {
            let frame = decoded_message(
                WireKind::LocalDeviceRevokeRequest,
                request_id,
                &v1::LocalDeviceRevokeRequest {
                    device_id: Some(device_id.into()),
                },
            );
            let service = self.service.clone();
            tokio::spawn(async move {
                let reply = service
                    .dispatch_until(frame, Instant::now() + Duration::from_secs(10))
                    .await;
                decode_one(&reply.bytes)
            })
        }

        fn finish(self, sessions: &SessionService) -> AuthorizationSnapshot {
            let paths = self.paths.clone();
            sessions
                .shutdown()
                .expect("matrix Session owners shut down");
            self.actor.shutdown();
            StateStore::open(&paths)
                .expect("reopen durable matrix state")
                .authorization_snapshot(MATRIX_REMOTE)
                .expect("read durable matrix revoke after StoreActor restart")
        }
    }

    fn test_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn empty_service(own: DeviceId) -> SessionService {
        SessionService::with_spawner(own, ResourceLimits::default(), |_size, _cwd| {
            panic!("empty Session wire tests must not spawn a PTY")
        })
    }

    fn remote_context(
        own: DeviceId,
        remote: DeviceId,
        accepted: AuthGeneration,
        authorization: AuthorizationRegistry,
    ) -> SessionRequestContext {
        SessionRequestContext::remote(own, remote, accepted, authorization)
            .expect("valid remote request context")
    }

    async fn require_first_poll_pending(
        operation: &'static str,
        expected_device: DeviceId,
        mut observation: tokio_mpsc::UnboundedReceiver<(DeviceId, bool)>,
    ) {
        let (device_id, pending) = tokio::time::timeout(Duration::from_secs(2), observation.recv())
            .await
            .unwrap_or_else(|_| panic!("{operation} did not reach its first lock poll"))
            .unwrap_or_else(|| panic!("{operation} first-poll observer was dropped"));
        assert_eq!(
            device_id, expected_device,
            "{operation} observed the wrong authorization owner",
        );
        assert!(
            pending,
            "{operation} submitted after the revoke writer must wait behind it",
        );
    }

    struct StalledFlushWriter;

    impl AsyncWrite for StalledFlushWriter {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
            bytes: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Ready(Ok(bytes.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    fn remote_target(device_id: DeviceId) -> v1::TargetSelector {
        v1::TargetSelector {
            target: Some(v1::target_selector::Target::Device(device_id.into())),
        }
    }

    fn decode_one(bytes: &[u8]) -> DecodedFrame {
        let mut decoder = FrameDecoder::new();
        let mut frames = decoder.feed(bytes).expect("decode response frame");
        decoder.finish().expect("complete response frame");
        assert_eq!(frames.len(), 1, "one stream response frame is expected");
        frames.remove(0)
    }

    fn decoded_message<Message: prost::Message>(
        kind: WireKind,
        request_id: u64,
        message: &Message,
    ) -> DecodedFrame {
        let bytes =
            encode_message(kind, request_id, 0, message).expect("encode bounded matrix request");
        decode_one(&bytes)
    }

    fn service_error_kind(bytes: &[u8]) -> DomainErrorKind {
        let frame = decode_one(bytes);
        let error: v1::ServiceError = frame
            .decode_message(WireKind::ServiceErrorResponse)
            .expect("typed service error");
        DomainErrorKind::from_code(&error.code).expect("stable domain error code")
    }

    struct TestRemotePeer {
        stream: DuplexStream,
        decoder: FrameDecoder,
        queued: VecDeque<DecodedFrame>,
    }

    impl TestRemotePeer {
        fn new(stream: DuplexStream) -> Self {
            Self {
                stream,
                decoder: FrameDecoder::new(),
                queued: VecDeque::new(),
            }
        }

        async fn send<Message: prost::Message>(
            &mut self,
            kind: WireKind,
            request_id: u64,
            message: &Message,
        ) {
            let bytes = encode_message(kind, request_id, 0, message)
                .expect("bounded remote duplex fixture frame");
            self.stream
                .write_all(&bytes)
                .await
                .expect("write remote duplex fixture frame");
            self.stream
                .flush()
                .await
                .expect("flush remote duplex fixture frame");
        }

        async fn next(&mut self) -> DecodedFrame {
            tokio::time::timeout(Duration::from_secs(2), async {
                if let Some(frame) = self.queued.pop_front() {
                    return frame;
                }
                let mut buffer = [0_u8; 16 * 1024];
                loop {
                    let read = self
                        .stream
                        .read(&mut buffer)
                        .await
                        .expect("read remote duplex fixture frame");
                    assert_ne!(read, 0, "remote duplex stream closed before its frame");
                    self.queued.extend(
                        self.decoder
                            .feed(&buffer[..read])
                            .expect("decode remote duplex fixture frame"),
                    );
                    if let Some(frame) = self.queued.pop_front() {
                        return frame;
                    }
                }
            })
            .await
            .expect("remote duplex fixture frame deadline")
        }
    }

    async fn start_remote_attachment(
        server: SessionWireServer,
        context: SessionRequestContext,
        request: v1::TerminalAttachRequest,
    ) -> (
        TestRemotePeer,
        tokio::task::JoinHandle<Result<(), DaemonError>>,
    ) {
        let (peer, service_stream) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            server
                .handle_remote_stream(
                    service_stream,
                    context,
                    SessionWireLimits::default(),
                    Instant::now() + Duration::from_secs(2),
                )
                .await
        });
        let mut peer = TestRemotePeer::new(peer);
        peer.send(WireKind::TerminalAttachRequest, 1, &request)
            .await;
        (peer, task)
    }

    fn remote_attach_request(
        own: DeviceId,
        session_id: SessionId,
        view_id: ResumeViewId,
        known_revision: Option<Revision>,
    ) -> v1::TerminalAttachRequest {
        v1::TerminalAttachRequest {
            target: Some(remote_target(own)),
            session_id: Some(session_id.into()),
            takeover: false,
            session_name: String::new(),
            create_main: false,
            viewport: None,
            resume_view_id: Some(view_id.into()),
            known_revision: known_revision.map(Revision::get),
        }
    }

    async fn acknowledge_and_barrier(
        peer: &mut TestRemotePeer,
        own: DeviceId,
        attachment_id: AttachmentId,
        revision: Revision,
    ) -> Revision {
        peer.send(
            WireKind::TerminalSnapshotApplied,
            2,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(attachment_id.into()),
                revision: revision.get(),
            },
        )
        .await;
        peer.send(
            WireKind::SessionOperationLeaseRequest,
            3,
            &v1::SessionOperationLeaseRequest {
                target: Some(remote_target(own)),
            },
        )
        .await;
        let mut latest_revision = revision;
        loop {
            let barrier = peer.next().await;
            if barrier.kind == WireKind::TerminalDelta {
                assert_eq!(barrier.request_id, 0);
                let delta: v1::TerminalDelta = barrier
                    .decode_message(WireKind::TerminalDelta)
                    .expect("decode live delta preceding the activation barrier");
                let delta_attachment: AttachmentId = delta
                    .attachment_id
                    .expect("live activation delta carries an attachment ID")
                    .try_into()
                    .expect("live activation delta attachment ID is valid");
                assert_eq!(delta_attachment, attachment_id);
                assert_eq!(delta.from_revision, latest_revision.get());
                assert!(delta.to_revision >= delta.from_revision);
                latest_revision = Revision::new(delta.to_revision);
                continue;
            }
            assert_eq!(barrier.kind, WireKind::SessionOperationLeaseResponse);
            assert_eq!(barrier.request_id, 3);
            let barrier: v1::SessionOperationLeaseResponse = barrier
                .decode_message(WireKind::SessionOperationLeaseResponse)
                .expect("decode remote activation barrier");
            let _: OperationLease = barrier
                .lease
                .expect("activation barrier returns a daemon-issued lease")
                .try_into()
                .expect("valid activation-barrier lease");
            return latest_revision;
        }
    }

    fn unix_wire_service(own: DeviceId, working_directory: PathBuf) -> SessionService {
        let cat = [Path::new("/bin/cat"), Path::new("/usr/bin/cat")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX cat fixture")
            .to_path_buf();
        SessionService::with_spawner(own, ResourceLimits::default(), move |size, requested| {
            let cwd = requested.unwrap_or(&working_directory).to_path_buf();
            let session = PtyHost::new()
                .spawn(
                    ExplicitPtyCommand::new(&cat, &cwd),
                    PtySize::new(size.rows, size.columns),
                )
                .map_err(|error| {
                    DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string())
                })?;
            Ok((session, cwd))
        })
    }

    fn unix_script_wire_service(
        own: DeviceId,
        working_directory: PathBuf,
        script: &'static str,
    ) -> SessionService {
        let shell = [Path::new("/bin/sh"), Path::new("/usr/bin/sh")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX shell fixture")
            .to_path_buf();
        SessionService::with_spawner(own, ResourceLimits::default(), move |size, requested| {
            let cwd = requested.unwrap_or(&working_directory).to_path_buf();
            let session = PtyHost::new()
                .spawn(
                    ExplicitPtyCommand::new(&shell, &cwd).arg("-c").arg(script),
                    PtySize::new(size.rows, size.columns),
                )
                .map_err(|error| {
                    DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string())
                })?;
            Ok((session, cwd))
        })
    }

    fn activate_attachment(prepared: &PreparedAttachment) {
        let replacement = prepared
            .attachment
            .snapshot_applied(prepared.snapshot.revision)
            .expect("activate matrix attachment");
        assert!(
            replacement.is_none(),
            "exact matrix snapshot acknowledgement cannot require replacement",
        );
    }

    async fn wait_for_attachment_text(prepared: &PreparedAttachment, expected: &[u8]) {
        let contains = |snapshot: &zterm_core::terminal::TerminalSnapshot| {
            snapshot
                .recent_history_ansi
                .windows(expected.len())
                .chain(snapshot.screen_ansi.windows(expected.len()))
                .any(|bytes| bytes == expected)
        };
        if contains(&prepared.snapshot) {
            return;
        }

        let mut revisions = prepared
            .attachment
            .revision_watch()
            .expect("history fixture revision watermark");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                revisions
                    .changed()
                    .await
                    .expect("history fixture driver remains live");
                let snapshot = prepared
                    .attachment
                    .sync_latest(prepared.snapshot.revision)
                    .expect("history fixture latest snapshot");
                if contains(&snapshot) {
                    assert!(
                        prepared
                            .attachment
                            .snapshot_applied(snapshot.revision)
                            .expect("acknowledge history fixture snapshot")
                            .is_none()
                    );
                    break;
                }
            }
        })
        .await
        .expect("history fixture output deadline");
    }

    fn session_summary(sessions: &SessionService, session_id: SessionId) -> SessionSummary {
        sessions
            .list()
            .expect("list matrix Sessions")
            .into_iter()
            .find(|summary| summary.session_id == session_id)
            .expect("matrix Session remains present")
    }

    async fn issue_remote_lease(
        server: SessionWireServer,
        context: SessionRequestContext,
        own: DeviceId,
        request_id: u64,
    ) -> OperationLease {
        let request = encode_message(
            WireKind::SessionOperationLeaseRequest,
            request_id,
            0,
            &v1::SessionOperationLeaseRequest {
                target: Some(remote_target(own)),
            },
        )
        .expect("encode matrix remote lease request");
        let (result, response) = run_remote_bytes(server, context, request).await;
        result.expect("matrix remote lease stream completes");
        let frame = decode_one(&response);
        assert_eq!(frame.request_id, request_id);
        assert_eq!(frame.kind, WireKind::SessionOperationLeaseResponse);
        let response: v1::SessionOperationLeaseResponse = frame
            .decode_message(WireKind::SessionOperationLeaseResponse)
            .expect("decode matrix operation lease");
        response
            .lease
            .expect("matrix host issued an operation lease")
            .try_into()
            .expect("valid matrix operation lease")
    }

    async fn run_remote_bytes(
        server: SessionWireServer,
        context: SessionRequestContext,
        request: Vec<u8>,
    ) -> (Result<(), DaemonError>, Vec<u8>) {
        let (mut client, service_stream) = tokio::io::duplex(64 * 1024);
        let deadline = Instant::now() + Duration::from_secs(1);
        let task = tokio::spawn(async move {
            server
                .handle_remote_stream(
                    service_stream,
                    context,
                    SessionWireLimits::default(),
                    deadline,
                )
                .await
        });
        client
            .write_all(&request)
            .await
            .expect("write remote request");
        client.shutdown().await.expect("finish remote request");
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .await
            .expect("read remote response");
        (
            task.await
                .expect("remote Session stream task did not panic"),
            response,
        )
    }

    #[tokio::test]
    async fn generic_first_read_retains_leftovers_and_strict_unary_rejects_them() {
        let first = encode_message(
            WireKind::SessionListRequest,
            41,
            0,
            &v1::SessionListRequest { target: None },
        )
        .expect("encode first frame");
        let trailing = encode_message(
            WireKind::SessionListRequest,
            42,
            0,
            &v1::SessionListRequest { target: None },
        )
        .expect("encode trailing frame");
        let (mut client, mut server) = tokio::io::duplex(4 * 1024);
        let mut request = first;
        request.extend_from_slice(&trailing);
        client.write_all(&request).await.expect("write both frames");
        client.shutdown().await.expect("finish duplex request");

        let first = read_first(&mut server).await.expect("read first frame");
        assert_eq!(first.frame.request_id, 41);
        assert_eq!(first.queued.len(), 1);
        let error = finish_unary(&mut server, first)
            .await
            .expect_err("a second frame must not pass strict unary EOF");
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
    }

    #[test]
    fn remote_context_rejects_self_and_zero_generation() {
        let own = device(0x0f);
        let authorization = AuthorizationRegistry::new();
        let self_error =
            match SessionRequestContext::remote(own, own, generation(1), authorization.clone()) {
                Ok(_) => panic!("the host identity cannot become a remote principal"),
                Err(error) => error,
            };
        assert_eq!(self_error.kind(), DomainErrorKind::Unauthorized);

        let zero_error = match SessionRequestContext::remote(
            own,
            device(0x10),
            AuthGeneration::ZERO,
            authorization,
        ) {
            Ok(_) => panic!("a remote principal requires a non-zero accepted generation"),
            Err(error) => error,
        };
        assert_eq!(zero_error.kind(), DomainErrorKind::Unauthorized);
    }

    #[test]
    fn only_authenticated_clean_eof_can_move_a_resume_checkpoint() {
        assert!(should_move_remote_resume_checkpoint(
            true,
            &Ok(AttachmentTaskEnd::TransportEof),
        ));
        assert!(!should_move_remote_resume_checkpoint(
            false,
            &Ok(AttachmentTaskEnd::TransportEof),
        ));
        assert!(!should_move_remote_resume_checkpoint(
            true,
            &Ok(AttachmentTaskEnd::Explicit),
        ));
        assert!(!should_move_remote_resume_checkpoint(
            true,
            &Err(DaemonError::new(
                DomainErrorKind::DaemonStopped,
                "transport I/O failed without clean EOF",
            )),
        ));
    }

    #[tokio::test]
    async fn attachment_flush_and_control_queue_are_deadline_bounded() {
        let mut writer = StalledFlushWriter;
        let write_error = write_attachment_bytes_until(
            &mut writer,
            b"bounded",
            Instant::now() + Duration::from_millis(10),
            "test attachment flush deadline",
            "test attachment flush",
        )
        .await
        .expect_err("a stalled attachment flush must release at its deadline");
        assert_eq!(write_error.kind(), DomainErrorKind::DeadlineExceeded);

        let (sender, mut receiver) = mpsc::channel(ATTACHMENT_OUTBOUND_CAPACITY);
        let future = Instant::now() + Duration::from_secs(1);
        for index in 0..ATTACHMENT_OUTBOUND_CAPACITY {
            let byte = u8::try_from(index).expect("outbound fixture index fits one byte");
            assert!(
                sender
                    .try_send(AttachmentOutbound::queued(vec![byte], future))
                    .is_ok(),
                "every production outbound slot is admitted",
            );
        }
        assert_eq!(sender.capacity(), 0);
        let queue_error = send_attachment_outbound_until(
            &sender,
            AttachmentOutbound::queued(vec![0xff], future),
            Instant::now(),
        )
        .await
        .expect_err("the next outbound response is backpressured at the exact bound");
        assert_eq!(queue_error.kind(), DomainErrorKind::DeadlineExceeded);

        receiver
            .recv()
            .await
            .expect("reap one queued outbound response");
        send_attachment_outbound_until(
            &sender,
            AttachmentOutbound::queued(vec![0xfe], future),
            future,
        )
        .await
        .expect("reaping one response recovers one outbound slot");
        assert_eq!(receiver.len(), ATTACHMENT_OUTBOUND_CAPACITY);
    }

    #[tokio::test]
    async fn shared_decoder_diagnostics_do_not_describe_remote_streams_as_local() {
        let own = device(0x11);
        let remote = device(0x12);
        let accepted = generation(2);
        let authorization = authorized_registry(remote, accepted);
        let (result, response) = run_remote_bytes(
            SessionWireServer::new(empty_service(own)),
            remote_context(own, remote, accepted, authorization),
            Vec::new(),
        )
        .await;
        let error = result.expect_err("EOF before the first frame is rejected");
        assert!(!error.detail().to_ascii_lowercase().contains("local"));

        let frame = decode_one(&response);
        let response: v1::ServiceError = frame
            .decode_message(WireKind::ServiceErrorResponse)
            .expect("typed service error");
        assert!(!response.message.to_ascii_lowercase().contains("local"));
    }

    #[tokio::test]
    async fn remote_target_is_exact_and_current_generation_dispatches_list() {
        let own = device(0x11);
        let remote = device(0x22);
        let accepted = generation(7);
        let authorization = authorized_registry(remote, accepted);
        let server = SessionWireServer::new(empty_service(own));

        let request = encode_message(
            WireKind::SessionListRequest,
            71,
            0,
            &v1::SessionListRequest {
                target: Some(remote_target(own)),
            },
        )
        .expect("encode remote list");
        let (result, response) = run_remote_bytes(
            server.clone(),
            remote_context(own, remote, accepted, authorization.clone()),
            request,
        )
        .await;
        result.expect("current generation list succeeds");
        let frame = decode_one(&response);
        assert_eq!(frame.kind, WireKind::SessionListResponse);
        let response: v1::SessionListResponse = frame
            .decode_message(WireKind::SessionListResponse)
            .expect("decode list response");
        assert!(response.sessions.is_empty());

        let wrong_target = encode_message(
            WireKind::SessionListRequest,
            72,
            0,
            &v1::SessionListRequest {
                target: Some(remote_target(device(0x33))),
            },
        )
        .expect("encode confused-deputy request");
        let (result, response) = run_remote_bytes(
            server.clone(),
            remote_context(own, remote, accepted, authorization.clone()),
            wrong_target,
        )
        .await;
        result.expect("typed target rejection is a complete unary response");
        assert_eq!(service_error_kind(&response), DomainErrorKind::Unauthorized);

        let local_target = encode_message(
            WireKind::SessionListRequest,
            73,
            0,
            &v1::SessionListRequest {
                target: Some(v1::TargetSelector {
                    target: Some(v1::target_selector::Target::Local(true)),
                }),
            },
        )
        .expect("encode local target over remote stream");
        let (result, response) = run_remote_bytes(
            server,
            remote_context(own, remote, accepted, authorization),
            local_target,
        )
        .await;
        result.expect("typed local-target rejection is a complete unary response");
        assert_eq!(service_error_kind(&response), DomainErrorKind::Unauthorized);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn same_uid_and_authenticated_remote_history_route_to_one_session_owner() {
        let temporary = tempfile::tempdir().expect("temporary history routing fixture");
        let own = device(0x19);
        let remote = device(0x1a);
        let accepted = generation(4);
        let sessions = unix_script_wire_service(
            own,
            temporary.path().to_path_buf(),
            "i=0; while [ \"$i\" -lt 12 ]; do printf 'history-%02d\\r\\n' \"$i\"; i=$((i + 1)); done; exec /bin/cat",
        );
        let local = sessions.local_principal(AttachmentId::from_array([0x1b; 16]));
        let lease = sessions
            .issue_operation_lease(local)
            .expect("history fixture create lease");
        let summary = sessions
            .create(
                local,
                OperationId { lease, sequence: 1 },
                SessionName::new("history-routing").expect("history fixture Session name"),
                None,
                Some(TerminalSize::new(3, 24)),
            )
            .expect("history fixture Session creates");
        let local_attachment = sessions
            .prepare_attach(
                local,
                Some(SessionSelector::Id(summary.session_id)),
                false,
                false,
                None,
            )
            .expect("same-UID attachment prepares");
        activate_attachment(&local_attachment);
        wait_for_attachment_text(&local_attachment, b"history-11").await;
        let local_page = local_attachment
            .attachment
            .history_page_until(
                TerminalHistoryDirection::Newest,
                None,
                8,
                Instant::now() + Duration::from_secs(2),
            )
            .expect("same-UID history request reaches the Session owner");
        let zterm_core::terminal::TerminalHistoryResult::Page(local_page) = local_page else {
            panic!("stable same-UID history returns a page");
        };
        assert!(!local_page.rows.is_empty());
        drop(local_attachment);

        let authorization = authorized_registry(remote, accepted);
        let (mut peer, task) = start_remote_attachment(
            SessionWireServer::new(sessions),
            remote_context(own, remote, accepted, authorization),
            remote_attach_request(
                own,
                summary.session_id,
                ResumeViewId::from_array([0x1c; 16]),
                None,
            ),
        )
        .await;
        let snapshot = peer.next().await;
        assert_eq!(snapshot.kind, WireKind::TerminalSnapshot);
        let snapshot: v1::TerminalSnapshot = snapshot
            .decode_message(WireKind::TerminalSnapshot)
            .expect("authenticated remote history snapshot");
        let attachment_id: AttachmentId = snapshot
            .attachment_id
            .expect("authenticated remote attachment ID")
            .try_into()
            .expect("fixed-width authenticated remote attachment ID");
        acknowledge_and_barrier(
            &mut peer,
            own,
            attachment_id,
            Revision::new(snapshot.revision),
        )
        .await;

        peer.send(
            WireKind::TerminalHistoryRequest,
            7,
            &v1::TerminalHistoryRequest {
                attachment_id: Some(attachment_id.into()),
                direction: v1::TerminalHistoryDirection::Newest as i32,
                cursor: None,
                maximum_rows: 8,
            },
        )
        .await;
        let page = peer.next().await;
        assert_eq!(page.kind, WireKind::TerminalHistoryPage);
        assert_eq!(page.request_id, 7);
        let page: v1::TerminalHistoryPage = page
            .decode_message(WireKind::TerminalHistoryPage)
            .expect("authenticated remote history page");
        assert_eq!(
            v1::TerminalHistoryOutcome::try_from(page.outcome).expect("known history outcome"),
            v1::TerminalHistoryOutcome::Ok
        );
        assert_eq!(page.rows, local_page.rows);

        peer.stream
            .shutdown()
            .await
            .expect("finish authenticated remote history fixture");
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("authenticated remote history server task deadline")
            .expect("authenticated remote history server task")
            .expect("authenticated remote history transport EOF is normal");
    }

    #[tokio::test]
    async fn stale_generation_cannot_list_issue_a_lease_or_prepare_attach() {
        let own = device(0x31);
        let remote = device(0x32);
        let current = generation(9);
        let stale = generation(8);
        let authorization = authorized_registry(remote, current);
        let sessions = empty_service(own);
        let server = SessionWireServer::new(sessions.clone());
        let context = remote_context(own, remote, stale, authorization.clone());

        for (kind, request) in [
            (
                WireKind::SessionListRequest,
                encode_message(
                    WireKind::SessionListRequest,
                    81,
                    0,
                    &v1::SessionListRequest {
                        target: Some(remote_target(own)),
                    },
                )
                .expect("encode stale list"),
            ),
            (
                WireKind::SessionOperationLeaseRequest,
                encode_message(
                    WireKind::SessionOperationLeaseRequest,
                    82,
                    0,
                    &v1::SessionOperationLeaseRequest {
                        target: Some(remote_target(own)),
                    },
                )
                .expect("encode stale lease"),
            ),
        ] {
            let (result, response) =
                run_remote_bytes(server.clone(), context.clone(), request).await;
            result.expect("typed stale-generation response completes");
            assert_eq!(
                service_error_kind(&response),
                DomainErrorKind::Unauthorized,
                "{kind:?} must acquire the current generation at its effect window"
            );
        }

        let attach = encode_message(
            WireKind::TerminalAttachRequest,
            83,
            0,
            &v1::TerminalAttachRequest {
                target: Some(remote_target(own)),
                session_id: None,
                takeover: false,
                session_name: String::new(),
                create_main: true,
                viewport: None,
                resume_view_id: None,
                known_revision: None,
            },
        )
        .expect("encode stale attach");
        let frame = decode_one(&attach);
        let error = match server
            .prepare_attachment(&context, &frame, Instant::now() + Duration::from_secs(1))
            .await
        {
            Ok(_) => panic!("stale prepare_attach cannot reach the PTY spawner"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), DomainErrorKind::Unauthorized);
    }

    #[tokio::test]
    async fn operation_lease_uses_the_remote_principal_generation() {
        let own = device(0x41);
        let remote = device(0x42);
        let first = generation(1);
        let authorization = authorized_registry(remote, first);
        let server = SessionWireServer::new(empty_service(own));
        let request = || {
            encode_message(
                WireKind::SessionOperationLeaseRequest,
                91,
                0,
                &v1::SessionOperationLeaseRequest {
                    target: Some(remote_target(own)),
                },
            )
            .expect("encode operation lease request")
        };

        let (_, response) = run_remote_bytes(
            server.clone(),
            remote_context(own, remote, first, authorization.clone()),
            request(),
        )
        .await;
        let frame = decode_one(&response);
        let lease: v1::SessionOperationLeaseResponse = frame
            .decode_message(WireKind::SessionOperationLeaseResponse)
            .expect("decode operation lease");
        assert!(lease.lease.is_some());

        {
            let mut writer = authorization
                .authorize_guard(remote)
                .await
                .expect("generation writer");
            writer
                .publish(AuthorizationSnapshot {
                    status: AuthorizationStatus::Authorized,
                    generation: generation(2),
                })
                .expect("publish next generation");
        }
        let (_, response) = run_remote_bytes(
            server,
            remote_context(own, remote, first, authorization),
            request(),
        )
        .await;
        assert_eq!(service_error_kind(&response), DomainErrorKind::Unauthorized);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_revoke_writer_orders_remote_effect_matrix_and_preserves_sessions() {
        let own = device(0x80);
        let pty_temporary = tempfile::tempdir().expect("temporary revoke-matrix PTY fixture");
        let sessions = unix_wire_service(own, pty_temporary.path().to_path_buf());
        let mut harness = DirectRevokeHarness::start(sessions.clone());
        let remote_before = harness
            .authorization
            .snapshot(MATRIX_REMOTE)
            .expect("matrix remote authorization snapshot");
        let other_before = harness
            .authorization
            .snapshot(MATRIX_OTHER)
            .expect("matrix other authorization snapshot");
        assert_eq!(remote_before.status, AuthorizationStatus::Authorized);
        assert_eq!(other_before.status, AuthorizationStatus::Authorized);
        let revoked = AuthorizationSnapshot {
            status: AuthorizationStatus::Revoked,
            generation: remote_before
                .generation
                .checked_next()
                .expect("matrix generation advances once"),
        };

        let local = sessions.local_principal(AttachmentId::from_array([0x83; 16]));
        let local_lease = sessions
            .issue_operation_lease(local)
            .expect("matrix local create lease");
        let initial_viewport = TerminalSize::new(24, 80);
        let primary = sessions
            .create(
                local,
                OperationId {
                    lease: local_lease,
                    sequence: 1,
                },
                SessionName::new("matrix-primary").expect("matrix primary name"),
                None,
                Some(initial_viewport),
            )
            .expect("matrix primary Session creates");
        let other = sessions
            .create(
                local,
                OperationId {
                    lease: local_lease,
                    sequence: 2,
                },
                SessionName::new("matrix-other").expect("matrix other name"),
                None,
                Some(initial_viewport),
            )
            .expect("matrix other Session creates");
        let server = SessionWireServer::new(sessions.clone());
        let matrix_remote_context = remote_context(
            own,
            MATRIX_REMOTE,
            remote_before.generation,
            harness.authorization.clone(),
        );
        let other_context = remote_context(
            own,
            MATRIX_OTHER,
            other_before.generation,
            harness.authorization.clone(),
        );

        let remote_attach = decoded_message(
            WireKind::TerminalAttachRequest,
            401,
            &remote_attach_request(
                own,
                primary.session_id,
                ResumeViewId::from_array([0x84; 16]),
                None,
            ),
        );
        let remote_prepared = server
            .prepare_attachment(
                &matrix_remote_context,
                &remote_attach,
                Instant::now() + Duration::from_secs(5),
            )
            .await
            .expect("authorized matrix remote attaches");
        activate_attachment(&remote_prepared);
        let remote_attachment = Arc::clone(&remote_prepared.attachment);
        let mut remote_lifecycle = remote_attachment
            .lifecycle_watch()
            .expect("matrix remote lifecycle watch");
        assert!(matches!(
            *remote_lifecycle.borrow_and_update(),
            AttachmentLifecycle::Active { .. }
        ));

        let other_attach = decoded_message(
            WireKind::TerminalAttachRequest,
            402,
            &remote_attach_request(
                own,
                other.session_id,
                ResumeViewId::from_array([0x85; 16]),
                None,
            ),
        );
        let other_prepared = server
            .prepare_attachment(
                &other_context,
                &other_attach,
                Instant::now() + Duration::from_secs(5),
            )
            .await
            .expect("unaffected matrix remote attaches");
        activate_attachment(&other_prepared);
        let other_attachment = Arc::clone(&other_prepared.attachment);
        assert!(session_summary(&sessions, primary.session_id).has_controller);
        assert!(session_summary(&sessions, other.session_id).has_controller);

        let remote_lease =
            issue_remote_lease(server.clone(), matrix_remote_context.clone(), own, 403).await;

        // This real input effect owns the old generation's read permit before
        // revoke starts. It must finish before the queued revoke writer can
        // persist or publish the next generation.
        let first_commit = harness
            .authorization
            .acquire_commit(MATRIX_REMOTE, remote_before.generation)
            .await
            .expect("matrix old-generation commit acquired");
        let old_effect_calls = Arc::new(AtomicUsize::new(0));
        let old_effect_calls_worker = Arc::clone(&old_effect_calls);
        let (old_entered_tx, mut old_entered_rx) = tokio_mpsc::unbounded_channel();
        let (release_old_tx, release_old_rx) = std_mpsc::channel();
        let old_attachment = Arc::clone(&remote_attachment);
        let old_effect = tokio::spawn(async move {
            first_commit
                .run(move || {
                    old_effect_calls_worker.fetch_add(1, Ordering::AcqRel);
                    old_entered_tx
                        .send(())
                        .expect("matrix old effect observer remains live");
                    release_old_rx
                        .recv()
                        .expect("matrix old effect release remains live");
                    old_attachment.write_input_until(
                        b"old-authorized\n",
                        Instant::now() + Duration::from_secs(5),
                    )
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), old_entered_rx.recv())
            .await
            .expect("matrix old effect reached its commit boundary")
            .expect("matrix old effect observer remains open");

        let revoke_task = harness.spawn_revoke(404, MATRIX_REMOTE);
        harness.wait_for_writer(MATRIX_REMOTE).await;

        // Queue every sensitive operation owner only after the fair revoke
        // writer's first real poll. Each future must report Pending on its
        // first poll, proving it cannot overtake the writer.
        let list_server = server.clone();
        let (list_observer, list_observed) = tokio_mpsc::unbounded_channel();
        let list_context = matrix_remote_context
            .clone()
            .with_commit_first_poll_observer(list_observer);
        let list_task = tokio::spawn(async move {
            list_server
                .dispatch_unary_until(
                    &decoded_message(
                        WireKind::SessionListRequest,
                        405,
                        &v1::SessionListRequest {
                            target: Some(remote_target(own)),
                        },
                    ),
                    list_context,
                    Instant::now() + Duration::from_secs(5),
                )
                .await
        });

        let attach_server = server.clone();
        let (attach_observer, attach_observed) = tokio_mpsc::unbounded_channel();
        let attach_context = matrix_remote_context
            .clone()
            .with_commit_first_poll_observer(attach_observer);
        let attach_task = tokio::spawn(async move {
            let frame = decoded_message(
                WireKind::TerminalAttachRequest,
                406,
                &remote_attach_request(
                    own,
                    primary.session_id,
                    ResumeViewId::from_array([0x86; 16]),
                    None,
                ),
            );
            attach_server
                .prepare_attachment(
                    &attach_context,
                    &frame,
                    Instant::now() + Duration::from_secs(5),
                )
                .await
        });

        let limits = SessionWireLimits::default();
        let (outbound, mut outbound_rx) = mpsc::channel(ATTACHMENT_OUTBOUND_CAPACITY);
        let input_server = server.clone();
        let (input_observer, input_observed) = tokio_mpsc::unbounded_channel();
        let input_context = matrix_remote_context
            .clone()
            .with_commit_first_poll_observer(input_observer);
        let input_attachment = Arc::clone(&remote_attachment);
        let input_outbound = outbound.clone();
        let input_task = tokio::spawn(async move {
            process_attachment_frame(
                decoded_message(
                    WireKind::TerminalInput,
                    407,
                    &v1::TerminalInput {
                        operation_id: Some(
                            OperationId {
                                lease: remote_lease,
                                sequence: 1,
                            }
                            .into(),
                        ),
                        attachment_id: Some(input_attachment.attachment_id().into()),
                        bytes: b"must-not-reach-pty".to_vec(),
                    },
                ),
                &input_server,
                &input_attachment,
                &input_context,
                &input_outbound,
                limits,
            )
            .await
        });

        let resize_server = server.clone();
        let (resize_observer, resize_observed) = tokio_mpsc::unbounded_channel();
        let resize_context = matrix_remote_context
            .clone()
            .with_commit_first_poll_observer(resize_observer);
        let resize_attachment = Arc::clone(&remote_attachment);
        let resize_outbound = outbound.clone();
        let resize_task = tokio::spawn(async move {
            process_attachment_frame(
                decoded_message(
                    WireKind::TerminalResize,
                    408,
                    &v1::TerminalResize {
                        operation_id: Some(
                            OperationId {
                                lease: remote_lease,
                                sequence: 2,
                            }
                            .into(),
                        ),
                        attachment_id: Some(resize_attachment.attachment_id().into()),
                        rows: 41,
                        columns: 101,
                    },
                ),
                &resize_server,
                &resize_attachment,
                &resize_context,
                &resize_outbound,
                limits,
            )
            .await
        });

        let takeover_server = server.clone();
        let (takeover_observer, takeover_observed) = tokio_mpsc::unbounded_channel();
        let takeover_context = matrix_remote_context
            .clone()
            .with_commit_first_poll_observer(takeover_observer);
        let takeover_attachment = Arc::clone(&remote_attachment);
        let takeover_outbound = outbound.clone();
        let takeover_task = tokio::spawn(async move {
            process_attachment_frame(
                decoded_message(
                    WireKind::SessionTakeoverRequest,
                    409,
                    &v1::SessionTakeoverRequest {
                        operation_id: Some(
                            OperationId {
                                lease: remote_lease,
                                sequence: 3,
                            }
                            .into(),
                        ),
                        target: Some(remote_target(own)),
                        session_id: Some(primary.session_id.into()),
                        attachment_id: Some(takeover_attachment.attachment_id().into()),
                    },
                ),
                &takeover_server,
                &takeover_attachment,
                &takeover_context,
                &takeover_outbound,
                limits,
            )
            .await
        });

        require_first_poll_pending("remote list", MATRIX_REMOTE, list_observed).await;
        require_first_poll_pending("remote attach", MATRIX_REMOTE, attach_observed).await;
        require_first_poll_pending("terminal input", MATRIX_REMOTE, input_observed).await;
        require_first_poll_pending("terminal resize", MATRIX_REMOTE, resize_observed).await;
        require_first_poll_pending("session takeover", MATRIX_REMOTE, takeover_observed).await;

        assert_eq!(
            harness
                .store
                .authorization_snapshot(MATRIX_REMOTE, default_store_deadline())
                .expect("durable matrix snapshot before old commit releases"),
            remote_before,
        );
        assert_eq!(
            harness
                .authorization
                .snapshot(MATRIX_REMOTE)
                .expect("memory matrix snapshot before old commit releases"),
            remote_before,
        );
        assert_eq!(harness.access.close_count(), 0);
        assert!(!revoke_task.is_finished());
        for pending in [
            list_task.is_finished(),
            attach_task.is_finished(),
            input_task.is_finished(),
            resize_task.is_finished(),
            takeover_task.is_finished(),
        ] {
            assert!(!pending, "a later remote effect overtook the revoke writer");
        }

        release_old_tx
            .send(())
            .expect("release matrix old-generation effect");
        tokio::time::timeout(Duration::from_secs(2), old_effect)
            .await
            .expect("matrix old-generation effect completes")
            .expect("matrix old-generation effect task does not panic")
            .expect("matrix old-generation input commits");
        assert_eq!(
            old_effect_calls.load(Ordering::Acquire),
            1,
            "the already-started input effect commits exactly once",
        );

        // The production revoke has now committed SQLite and published memory,
        // but the fake close owner holds it before remote detach. This exposes
        // the required durable -> memory -> close -> detach ordering.
        let close = harness.access.wait_for_close(0).await;
        assert_eq!(close.device_id, MATRIX_REMOTE);
        assert_eq!(close.durable, revoked);
        assert_eq!(close.memory, revoked);
        assert_eq!(close.attachments_before_detach, 1);
        assert_eq!(close.sessions_before_detach, 2);
        assert_eq!(harness.access.close_count(), 1);
        assert!(!revoke_task.is_finished());
        assert!(
            remote_attachment
                .write_input_until(
                    b"close-before-detach\n",
                    Instant::now() + Duration::from_secs(2),
                )
                .is_ok(),
            "remote attachment stays live until the ordered close step returns",
        );
        for pending in [
            list_task.is_finished(),
            attach_task.is_finished(),
            input_task.is_finished(),
            resize_task.is_finished(),
            takeover_task.is_finished(),
        ] {
            assert!(!pending, "the revoke writer released before remote detach");
        }

        harness.access.release_first_close();
        let revoke_frame = tokio::time::timeout(Duration::from_secs(2), revoke_task)
            .await
            .expect("matrix revoke completes after close releases")
            .expect("matrix revoke task does not panic");
        assert_eq!(revoke_frame.kind, WireKind::LocalDeviceRevokeResponse);
        let revoke_response: v1::LocalDeviceRevokeResponse = revoke_frame
            .decode_message(WireKind::LocalDeviceRevokeResponse)
            .expect("decode matrix revoke response");
        let revoked_device: DeviceSummary = revoke_response
            .device
            .expect("matrix revoke returns a device summary")
            .try_into()
            .expect("valid matrix device summary");
        assert_eq!(revoked_device.device_id(), MATRIX_REMOTE);
        assert_eq!(revoked_device.auth_status(), AuthorizationStatus::Revoked);
        assert_eq!(revoked_device.generation(), revoked.generation);
        assert_eq!(revoked_device.remote_attachment_count(), 0);

        let list_reply = tokio::time::timeout(Duration::from_secs(2), list_task)
            .await
            .expect("queued list completes after revoke")
            .expect("queued list task does not panic");
        assert_eq!(
            service_error_kind(&list_reply.bytes),
            DomainErrorKind::Unauthorized,
        );
        let attach_error = match tokio::time::timeout(Duration::from_secs(2), attach_task)
            .await
            .expect("queued attach completes after revoke")
            .expect("queued attach task does not panic")
        {
            Ok(_) => panic!("queued remote attach cannot allocate after revoke"),
            Err(error) => error,
        };
        assert_eq!(attach_error.kind(), DomainErrorKind::Unauthorized);
        for (operation, task) in [
            ("terminal input", input_task),
            ("terminal resize", resize_task),
            ("session takeover", takeover_task),
        ] {
            let error = tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap_or_else(|_| panic!("queued {operation} completes after revoke"))
                .unwrap_or_else(|_| panic!("queued {operation} task does not panic"))
                .expect_err("queued remote effect cannot commit after revoke");
            assert_eq!(error.kind(), DomainErrorKind::Unauthorized);
        }
        assert!(
            matches!(
                outbound_rx.try_recv(),
                Err(tokio_mpsc::error::TryRecvError::Empty)
            ),
            "denied takeover emits no success response",
        );
        drop(outbound);

        assert_eq!(
            sessions
                .remote_attachment_count_until(MATRIX_REMOTE, default_store_deadline())
                .expect("count revoked matrix attachments"),
            0,
        );
        assert_eq!(
            sessions
                .remote_attachment_count_until(MATRIX_OTHER, default_store_deadline())
                .expect("count unaffected matrix attachments"),
            1,
        );
        assert_eq!(
            remote_attachment
                .write_input_until(b"must-stay-stale", Instant::now() + Duration::from_secs(2),)
                .expect_err("revoked attachment cannot write")
                .kind(),
            DomainErrorKind::LeaseLost,
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(2), remote_lifecycle.changed())
                .await
                .expect("revoked attachment lifecycle owner retires")
                .is_err(),
            "detached remote lifecycle channel closes",
        );
        other_attachment
            .write_input_until(
                b"unaffected-principal\n",
                Instant::now() + Duration::from_secs(2),
            )
            .expect("unaffected remote attachment remains live");

        let primary_after = session_summary(&sessions, primary.session_id);
        let other_after = session_summary(&sessions, other.session_id);
        assert!(!primary_after.has_controller);
        assert_eq!(primary_after.viewport, initial_viewport);
        assert!(other_after.has_controller);
        assert_eq!(
            sessions.list().expect("matrix Sessions remain live").len(),
            2
        );
        assert_eq!(
            harness
                .authorization
                .snapshot(MATRIX_REMOTE)
                .expect("revoked matrix memory snapshot"),
            revoked,
        );
        assert_eq!(
            harness
                .authorization
                .snapshot(MATRIX_OTHER)
                .expect("unaffected matrix memory snapshot"),
            other_before,
        );
        assert_eq!(harness.access.close_count(), 1);

        let local_continuation = sessions
            .prepare_attach(
                local,
                Some(SessionSelector::Id(primary.session_id)),
                false,
                false,
                None,
            )
            .expect("same-UID client can continue the preserved Session");
        activate_attachment(&local_continuation);
        local_continuation
            .attachment
            .write_input_until(
                b"local-continuation\n",
                Instant::now() + Duration::from_secs(2),
            )
            .expect("preserved PTY accepts a new local controller");

        let durable_after_restart = harness.finish(&sessions);
        assert_eq!(durable_after_restart, revoked);
        drop(pty_temporary);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticated_duplex_eof_moves_checkpoint_but_explicit_and_protocol_end_do_not() {
        let temporary = tempfile::tempdir().expect("temporary Session wire checkpoint fixture");
        let own = device(0x43);
        let remote = device(0x44);
        let accepted = generation(5);
        let authorization = authorized_registry(remote, accepted);
        let sessions = unix_wire_service(own, temporary.path().to_path_buf());
        let local = sessions.local_principal(AttachmentId::from_array([0x45; 16]));
        let lease = sessions
            .issue_operation_lease(local)
            .expect("checkpoint fixture create lease");
        let summary = sessions
            .create(
                local,
                OperationId { lease, sequence: 1 },
                SessionName::new("wire-checkpoint").expect("fixture session name"),
                None,
                None,
            )
            .expect("checkpoint fixture session creates");
        let server = SessionWireServer::new(sessions.clone());
        let context = remote_context(own, remote, accepted, authorization);
        let view_id = ResumeViewId::from_array([0x46; 16]);

        let (mut first, first_task) = start_remote_attachment(
            server.clone(),
            context.clone(),
            remote_attach_request(own, summary.session_id, view_id, None),
        )
        .await;
        let first_snapshot = first.next().await;
        assert_eq!(first_snapshot.kind, WireKind::TerminalSnapshot);
        let first_snapshot: v1::TerminalSnapshot = first_snapshot
            .decode_message(WireKind::TerminalSnapshot)
            .expect("initial remote snapshot");
        let first_attachment: AttachmentId = first_snapshot
            .attachment_id
            .clone()
            .expect("initial attachment ID")
            .try_into()
            .expect("fixed-width initial attachment ID");
        let baseline = acknowledge_and_barrier(
            &mut first,
            own,
            first_attachment,
            Revision::new(first_snapshot.revision),
        )
        .await;
        first
            .stream
            .shutdown()
            .await
            .expect("transport EOF after active authentication");
        tokio::time::timeout(Duration::from_secs(2), first_task)
            .await
            .expect("transport EOF server task completes")
            .expect("transport EOF server task")
            .expect("transport EOF is a normal attachment end");

        let (mut resumed, resumed_task) = start_remote_attachment(
            server.clone(),
            context.clone(),
            remote_attach_request(own, summary.session_id, view_id, Some(baseline)),
        )
        .await;
        let resumed_delta = resumed.next().await;
        assert_eq!(
            resumed_delta.kind,
            WireKind::TerminalDelta,
            "authenticated transport EOF moves the one exact checkpoint"
        );
        let resumed_delta: v1::TerminalDelta = resumed_delta
            .decode_message(WireKind::TerminalDelta)
            .expect("exact resumed delta");
        let resumed_attachment: AttachmentId = resumed_delta
            .attachment_id
            .clone()
            .expect("resumed attachment ID")
            .try_into()
            .expect("fixed-width resumed attachment ID");
        assert_ne!(resumed_attachment, first_attachment);
        assert_eq!(resumed_delta.from_revision, baseline.get());
        let resumed_revision = acknowledge_and_barrier(
            &mut resumed,
            own,
            resumed_attachment,
            Revision::new(resumed_delta.to_revision),
        )
        .await;
        resumed
            .send(
                WireKind::TerminalInput,
                4,
                &v1::TerminalInput {
                    operation_id: None,
                    attachment_id: Some(AttachmentId::from_array([0xff; 16]).into()),
                    bytes: b"must-not-reach-pty".to_vec(),
                },
            )
            .await;
        let protocol_error = resumed.next().await;
        assert_eq!(protocol_error.kind, WireKind::ServiceErrorResponse);
        assert_eq!(protocol_error.request_id, 4);
        let protocol_error: v1::ServiceError = protocol_error
            .decode_message(WireKind::ServiceErrorResponse)
            .expect("typed protocol failure");
        assert_eq!(
            DomainErrorKind::from_code(&protocol_error.code),
            Some(DomainErrorKind::MalformedFrame)
        );
        let protocol_result = tokio::time::timeout(Duration::from_secs(2), resumed_task)
            .await
            .expect("protocol-failure server task completes")
            .expect("protocol-failure server task")
            .expect_err("protocol failure remains stream-local and typed");
        assert_eq!(protocol_result.kind(), DomainErrorKind::MalformedFrame);

        let (mut after_protocol, after_protocol_task) = start_remote_attachment(
            server.clone(),
            context.clone(),
            remote_attach_request(own, summary.session_id, view_id, Some(resumed_revision)),
        )
        .await;
        let after_protocol_snapshot = after_protocol.next().await;
        assert_eq!(
            after_protocol_snapshot.kind,
            WireKind::TerminalSnapshot,
            "protocol failure discards rather than saves the live checkpoint"
        );
        let after_protocol_snapshot: v1::TerminalSnapshot = after_protocol_snapshot
            .decode_message(WireKind::TerminalSnapshot)
            .expect("authoritative snapshot after protocol failure");
        let after_protocol_attachment: AttachmentId = after_protocol_snapshot
            .attachment_id
            .clone()
            .expect("post-protocol attachment ID")
            .try_into()
            .expect("fixed-width post-protocol attachment ID");
        let after_protocol_revision = acknowledge_and_barrier(
            &mut after_protocol,
            own,
            after_protocol_attachment,
            Revision::new(after_protocol_snapshot.revision),
        )
        .await;
        after_protocol
            .send(
                WireKind::TerminalDetach,
                4,
                &v1::TerminalDetach {
                    attachment_id: Some(after_protocol_attachment.into()),
                },
            )
            .await;
        tokio::time::timeout(Duration::from_secs(2), after_protocol_task)
            .await
            .expect("explicit-detach server task completes")
            .expect("explicit-detach server task")
            .expect("explicit detach is a normal attachment end");

        let (mut after_explicit, after_explicit_task) = start_remote_attachment(
            server,
            context,
            remote_attach_request(
                own,
                summary.session_id,
                view_id,
                Some(after_protocol_revision),
            ),
        )
        .await;
        let after_explicit_snapshot = after_explicit.next().await;
        assert_eq!(
            after_explicit_snapshot.kind,
            WireKind::TerminalSnapshot,
            "explicit detach never creates a resume cell"
        );
        let after_explicit_snapshot: v1::TerminalSnapshot = after_explicit_snapshot
            .decode_message(WireKind::TerminalSnapshot)
            .expect("authoritative snapshot after explicit detach");
        after_explicit_snapshot
            .attachment_id
            .expect("authoritative snapshot carries a final attachment ID");
        // The prior stream already proved explicit detach. Leave this
        // observation-only reconnect unsynchronized and close its transport;
        // an unsynchronized attachment cannot create a resume checkpoint.
        after_explicit
            .stream
            .shutdown()
            .await
            .expect("finish final cleanup transport");
        tokio::time::timeout(Duration::from_secs(2), after_explicit_task)
            .await
            .expect("cleanup EOF server task completes")
            .expect("cleanup EOF server task")
            .expect("cleanup EOF succeeds");
        sessions.shutdown().expect("checkpoint fixture shuts down");
    }

    #[tokio::test]
    async fn trailing_malformed_and_stalled_remote_streams_are_isolated() {
        let own = device(0x51);
        let remote = device(0x52);
        let accepted = generation(3);
        let authorization = authorized_registry(remote, accepted);
        let server = SessionWireServer::new(empty_service(own));
        let context = remote_context(own, remote, accepted, authorization.clone());
        let request = encode_message(
            WireKind::SessionListRequest,
            101,
            0,
            &v1::SessionListRequest {
                target: Some(remote_target(own)),
            },
        )
        .expect("encode healthy list");

        let mut trailing = request.clone();
        trailing.extend_from_slice(&request);
        let (error, response) = run_remote_bytes(server.clone(), context.clone(), trailing).await;
        assert_eq!(
            error
                .expect_err("trailing unary frame is stream-local")
                .kind(),
            DomainErrorKind::MalformedFrame
        );
        assert_eq!(
            service_error_kind(&response),
            DomainErrorKind::MalformedFrame
        );

        let (error, response) =
            run_remote_bytes(server.clone(), context.clone(), vec![0x80, 0x00]).await;
        assert_eq!(
            error.expect_err("malformed prefix is stream-local").kind(),
            DomainErrorKind::MalformedFrame
        );
        assert_eq!(
            service_error_kind(&response),
            DomainErrorKind::MalformedFrame
        );

        let (_client, stalled) = tokio::io::duplex(64);
        let error = server
            .handle_remote_stream(
                stalled,
                context.clone(),
                SessionWireLimits::default(),
                Instant::now(),
            )
            .await
            .expect_err("elapsed first-frame deadline is stream-local");
        assert_eq!(error.kind(), DomainErrorKind::DeadlineExceeded);

        let (healthy, response) = run_remote_bytes(server, context, request).await;
        healthy.expect("independent healthy stream remains usable");
        assert_eq!(decode_one(&response).kind, WireKind::SessionListResponse);
    }
}
