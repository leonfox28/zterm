//! Daemon-owned desired-view bridge for reconnectable remote terminal streams.
//!
//! One bridge owns one same-UID local view and one connection demand. Remote
//! service streams and host-issued attachment identities are epoch-local; the
//! local view identity remains stable until explicit detach or a terminal
//! service outcome.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use iroh::SecretKey;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf, ReadHalf, WriteHalf};
use zterm_core::terminal::MAX_HISTORY_PAGE_ROWS;
use zterm_core::{
    AttachmentId, Capabilities, DeviceId, DomainErrorKind, ResumeViewId, Revision, SessionId,
};
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v1};

use crate::connection_broker::{
    AuthenticatedBiStream, ConnectionBroker, ConnectionDemand, SelectedCandidateObserver,
    SelectedPathObservation, StreamPurpose,
};
use crate::error::DaemonError;
use crate::network::PathKind;
use crate::remote_session::{
    SessionUnaryResponseStatus, decode_session_service_error, session_summary_from_wire,
    validate_session_unary_response,
};
use crate::service::{ServiceReply, protocol_error};
use crate::session_wire::{FirstFrame, SessionWireLimits};

const MAX_PENDING_CONTROL_REQUESTS: usize = 8;
const RESUME_OCCUPIED_RETRY_DELAY: Duration = Duration::from_millis(250);

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub(super) trait AsyncAttachmentStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> AsyncAttachmentStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub(super) type BoxAttachmentStream = Box<dyn AsyncAttachmentStream>;

trait RemoteAttachmentEpochObserver: Send + Sync {
    fn selected_path_observation(&self) -> SelectedPathObservation;

    fn supports(&self, capability: u64) -> bool;
}

pub(super) struct OpenedAttachmentEpoch {
    stream: BoxAttachmentStream,
    observer: Arc<dyn RemoteAttachmentEpochObserver>,
}

#[cfg(test)]
impl OpenedAttachmentEpoch {
    pub(super) fn unobserved(stream: BoxAttachmentStream) -> Self {
        Self {
            stream,
            observer: Arc::new(UnobservedAttachmentEpoch),
        }
    }
}

#[cfg(test)]
struct UnobservedAttachmentEpoch;

#[cfg(test)]
impl RemoteAttachmentEpochObserver for UnobservedAttachmentEpoch {
    fn selected_path_observation(&self) -> SelectedPathObservation {
        SelectedPathObservation::default()
    }

    fn supports(&self, _capability: u64) -> bool {
        false
    }
}

pub(super) trait RemoteAttachmentTransport: Send + Sync {
    fn demand<'a>(
        &'a self,
        target: DeviceId,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Box<dyn RemoteAttachmentDemand>, DaemonError>>;
}

pub(super) trait RemoteAttachmentDemand: Send + Sync {
    fn open<'a>(
        &'a mut self,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<OpenedAttachmentEpoch, DaemonError>>;
}

/// One reconnecting daemon-side attachment client.
#[derive(Clone)]
pub(crate) struct RemoteAttachmentClient {
    transport: Arc<dyn RemoteAttachmentTransport>,
}

impl RemoteAttachmentClient {
    pub(crate) fn production(broker: ConnectionBroker) -> Self {
        Self {
            transport: Arc::new(BrokerAttachmentTransport { broker }),
        }
    }

    #[cfg(all(test, unix))]
    pub(super) fn with_test_transport(transport: Arc<dyn RemoteAttachmentTransport>) -> Self {
        Self { transport }
    }

    pub(crate) async fn serve<LocalStream>(
        &self,
        target: DeviceId,
        local_view_id: AttachmentId,
        mut local_stream: LocalStream,
        first: FirstFrame,
        limits: SessionWireLimits,
        initial_deadline: Instant,
    ) -> Result<(), DaemonError>
    where
        LocalStream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let request_id = first.frame.request_id;
        let request: v1::TerminalAttachRequest = match first
            .frame
            .decode_message(WireKind::TerminalAttachRequest)
            .map_err(protocol_error)
        {
            Ok(request) => request,
            Err(error) => {
                write_error_best_effort(&mut local_stream, request_id, &error, initial_deadline)
                    .await;
                return Err(error);
            }
        };
        if let Err(error) = require_initial_target(&request, target) {
            write_error_best_effort(&mut local_stream, request_id, &error, initial_deadline).await;
            return Err(error);
        }
        if request.resume_view_id.is_some() || request.known_revision.is_some() {
            let error = malformed("same-UID callers cannot choose the daemon resume-view identity");
            write_error_best_effort(&mut local_stream, request_id, &error, initial_deadline).await;
            return Err(error);
        }
        let latest_viewport = match request
            .viewport
            .map(|viewport| {
                zterm_core::terminal::TerminalSize::try_from(viewport)
                    .map(|_| viewport)
                    .map_err(protocol_error)
            })
            .transpose()
        {
            Ok(viewport) => viewport,
            Err(error) => {
                write_error_best_effort(&mut local_stream, request_id, &error, initial_deadline)
                    .await;
                return Err(error);
            }
        };

        let (local_reader, mut local_writer) = tokio::io::split(local_stream);
        let mut local_reader = FramedReader::from_first(local_reader, first);
        let resume_view_id = random_resume_view_id();
        write_transport_state(
            &mut local_writer,
            local_view_id,
            v1::TerminalTransportState::Preparing,
            initial_deadline,
        )
        .await?;

        let mut state = BridgeState {
            request,
            request_id,
            target,
            local_view_id,
            resume_view_id,
            frozen_session_id: None,
            known_revision: None,
            latest_viewport,
            force_full: false,
            ever_active: false,
            pending_control: BTreeMap::new(),
        };
        let demand = timeout_at(
            initial_deadline,
            self.transport.demand(target, initial_deadline),
            "remote attachment demand exceeded its absolute deadline",
        );
        tokio::pin!(demand);
        let demand = loop {
            tokio::select! {
                result = &mut demand => break result.and_then(|result| result),
                local = local_reader.next() => {
                    match local? {
                        Some(frame) => {
                            if process_offline_local_frame(
                                frame,
                                &mut state,
                                &mut local_writer,
                                limits.operation_timeout(),
                            ).await? {
                                return Ok(());
                            }
                        }
                        None => return Ok(()),
                    }
                }
            }
        };
        let mut demand = match demand {
            Ok(demand) => demand,
            Err(error) => {
                write_error_best_effort(&mut local_writer, request_id, &error, initial_deadline)
                    .await;
                return Err(error);
            }
        };
        let result = run_bridge(
            &mut *demand,
            &mut local_reader,
            &mut local_writer,
            state,
            limits,
        )
        .await;
        if let Err(error) = &result {
            write_error_best_effort(
                &mut local_writer,
                request_id,
                error,
                Instant::now() + limits.operation_timeout(),
            )
            .await;
        }
        result
    }
}

struct BrokerAttachmentTransport {
    broker: ConnectionBroker,
}

impl RemoteAttachmentTransport for BrokerAttachmentTransport {
    fn demand<'a>(
        &'a self,
        target: DeviceId,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Box<dyn RemoteAttachmentDemand>, DaemonError>> {
        Box::pin(async move {
            let demand = self.broker.demand(target, deadline).await?;
            Ok(Box::new(BrokerAttachmentDemand { demand }) as Box<dyn RemoteAttachmentDemand>)
        })
    }
}

struct BrokerAttachmentDemand {
    demand: ConnectionDemand,
}

impl RemoteAttachmentDemand for BrokerAttachmentDemand {
    fn open<'a>(
        &'a mut self,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<OpenedAttachmentEpoch, DaemonError>> {
        Box::pin(async move {
            let stream = self
                .demand
                .open_bi(StreamPurpose::Service, deadline)
                .await?;
            let observer = Arc::new(stream.candidate_observer());
            Ok(OpenedAttachmentEpoch {
                stream: Box::new(BrokerAttachmentStream { stream }) as BoxAttachmentStream,
                observer,
            })
        })
    }
}

impl RemoteAttachmentEpochObserver for SelectedCandidateObserver {
    fn selected_path_observation(&self) -> SelectedPathObservation {
        self.selected_path_observation()
    }

    fn supports(&self, capability: u64) -> bool {
        self.supports(capability)
    }
}

struct BrokerAttachmentStream {
    stream: AuthenticatedBiStream,
}

impl AsyncRead for BrokerAttachmentStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream.recv).poll_read(context, buffer)
    }
}

impl AsyncWrite for BrokerAttachmentStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.stream.send), context, bytes)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.stream.send), context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.stream.send), context)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DesiredViewPhase {
    Preparing,
    Synchronizing,
    Active,
    Reconnecting,
}

struct BridgeState {
    request: v1::TerminalAttachRequest,
    request_id: u64,
    target: DeviceId,
    local_view_id: AttachmentId,
    resume_view_id: ResumeViewId,
    frozen_session_id: Option<SessionId>,
    known_revision: Option<Revision>,
    latest_viewport: Option<v1::TerminalViewport>,
    force_full: bool,
    ever_active: bool,
    pending_control: BTreeMap<u64, WireKind>,
}

struct RemoteEpoch {
    reader: FramedReader<ReadHalf<BoxAttachmentStream>>,
    writer: WriteHalf<BoxAttachmentStream>,
    observer: Arc<dyn RemoteAttachmentEpochObserver>,
    attachment_id: AttachmentId,
    initial_revision: Revision,
}

struct FramedReader<Reader> {
    reader: Reader,
    decoder: FrameDecoder,
    queued: VecDeque<DecodedFrame>,
}

impl<Reader> FramedReader<Reader>
where
    Reader: AsyncRead + Unpin,
{
    fn from_first(reader: Reader, first: FirstFrame) -> Self {
        Self {
            reader,
            decoder: first.decoder,
            queued: first.queued,
        }
    }

    fn fresh(reader: Reader) -> Self {
        Self {
            reader,
            decoder: FrameDecoder::new(),
            queued: VecDeque::new(),
        }
    }

    async fn next(&mut self) -> Result<Option<DecodedFrame>, DaemonError> {
        if let Some(frame) = self.queued.pop_front() {
            return Ok(Some(frame));
        }
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = self
                .reader
                .read(&mut buffer)
                .await
                .map_err(|_| transport_unavailable("terminal stream read failed"))?;
            if read == 0 {
                std::mem::replace(&mut self.decoder, FrameDecoder::new())
                    .finish()
                    .map_err(protocol_error)?;
                return Ok(None);
            }
            self.queued
                .extend(self.decoder.feed(&buffer[..read]).map_err(protocol_error)?);
            if let Some(frame) = self.queued.pop_front() {
                return Ok(Some(frame));
            }
        }
    }

    async fn next_until(
        &mut self,
        deadline: Instant,
        detail: &'static str,
    ) -> Result<Option<DecodedFrame>, DaemonError> {
        timeout_at(deadline, self.next(), detail).await?
    }
}

async fn run_bridge<LocalReader, LocalWriter>(
    demand: &mut dyn RemoteAttachmentDemand,
    local_reader: &mut FramedReader<LocalReader>,
    local_writer: &mut LocalWriter,
    mut state: BridgeState,
    limits: SessionWireLimits,
) -> Result<(), DaemonError>
where
    LocalReader: AsyncRead + Unpin,
    LocalWriter: AsyncWrite + Unpin,
{
    let mut phase = DesiredViewPhase::Preparing;
    'bridge: loop {
        let operation_timeout = limits.operation_timeout();
        let attempt_deadline = Instant::now() + operation_timeout;
        let remote = {
            let open = timeout_at(
                attempt_deadline,
                demand.open(attempt_deadline),
                "remote attachment open exceeded its absolute deadline",
            );
            tokio::pin!(open);
            loop {
                tokio::select! {
                    opened = &mut open => break opened.and_then(|opened| opened),
                    local = local_reader.next() => {
                        match local? {
                            Some(frame) => {
                                if process_offline_local_frame(
                                    frame,
                                    &mut state,
                                    local_writer,
                                    operation_timeout,
                                ).await? {
                                    return Ok(());
                                }
                            }
                            None => return Ok(()),
                        }
                    }
                }
            }
        };

        let remote = match remote {
            Ok(remote) => remote,
            Err(error) if is_temporary_transport(error.kind()) => {
                if phase != DesiredViewPhase::Reconnecting {
                    phase = DesiredViewPhase::Reconnecting;
                    write_transport_state(
                        local_writer,
                        state.local_view_id,
                        v1::TerminalTransportState::Reconnecting,
                        Instant::now() + operation_timeout,
                    )
                    .await?;
                }
                continue;
            }
            Err(error) => return Err(error),
        };

        let OpenedAttachmentEpoch {
            stream: remote,
            observer,
        } = remote;
        let (remote_reader, mut remote_writer) = tokio::io::split(remote);
        let attach = remote_attach_request(&state);
        let attach = encode_message(
            WireKind::TerminalAttachRequest,
            state.request_id,
            0,
            &attach,
        )
        .map_err(protocol_error)?;
        let attach_result = {
            let attach_write = write_remote(&mut remote_writer, &attach, attempt_deadline);
            tokio::pin!(attach_write);
            loop {
                tokio::select! {
                    result = &mut attach_write => break result,
                    local = local_reader.next() => {
                        match local? {
                            Some(frame) => {
                                if process_offline_local_frame(
                                    frame,
                                    &mut state,
                                    local_writer,
                                    operation_timeout,
                                ).await? {
                                    return Ok(());
                                }
                            }
                            None => return Ok(()),
                        }
                    }
                }
            }
        };
        if let Err(error) = attach_result {
            if is_temporary_transport(error.kind()) {
                enter_reconnecting(local_writer, &state, &mut phase, operation_timeout).await?;
                continue;
            }
            return Err(error);
        }

        let mut remote_reader = FramedReader::fresh(remote_reader);
        let initial = loop {
            tokio::select! {
                remote = remote_reader.next_until(
                    attempt_deadline,
                    "remote initial terminal update exceeded its absolute deadline",
                ) => break remote,
                local = local_reader.next() => {
                    match local? {
                        Some(frame) => {
                            if process_offline_local_frame(
                                frame,
                                &mut state,
                                local_writer,
                                operation_timeout,
                            ).await? {
                                let _ = send_remote_detach(
                                    &mut remote_writer,
                                    None,
                                    attempt_deadline,
                                ).await;
                                return Ok(());
                            }
                        }
                        None => {
                            let _ = send_remote_detach(
                                &mut remote_writer,
                                None,
                                attempt_deadline,
                            ).await;
                            return Ok(());
                        }
                    }
                }
            }
        };
        let Some(initial) = (match initial {
            Ok(frame) => frame,
            Err(error) if is_temporary_transport(error.kind()) => {
                enter_reconnecting(local_writer, &state, &mut phase, operation_timeout).await?;
                continue;
            }
            Err(error) => return Err(error),
        }) else {
            enter_reconnecting(local_writer, &state, &mut phase, operation_timeout).await?;
            continue;
        };

        let initial_request_id = state.request_id;
        let initial = accept_initial_remote_update(initial, &mut state, None, initial_request_id);
        let accepted = match initial {
            Err(error) if should_retry_resume_occupied(&state, &error) => {
                drop(remote_reader);
                drop(remote_writer);
                enter_reconnecting(local_writer, &state, &mut phase, operation_timeout).await?;
                if wait_before_resume_occupied_retry(
                    local_reader,
                    local_writer,
                    &mut state,
                    operation_timeout,
                )
                .await?
                {
                    return Ok(());
                }
                continue;
            }
            Err(error) => return Err(error),
            Ok(initial) => match initial {
                InitialRemoteUpdate::Accepted(accepted) => accepted,
                InitialRemoteUpdate::RequiresFull {
                    remote_attachment_id,
                    known_revision,
                } => {
                    state.force_full = true;
                    let request = v1::TerminalSyncRequest {
                        attachment_id: Some(remote_attachment_id.into()),
                        known_revision: known_revision.get(),
                    };
                    let bytes = encode_message(
                        WireKind::TerminalSyncRequest,
                        initial_request_id,
                        0,
                        &request,
                    )
                    .map_err(protocol_error)?;
                    {
                        let write = write_remote(&mut remote_writer, &bytes, attempt_deadline);
                        tokio::pin!(write);
                        loop {
                            tokio::select! {
                                result = &mut write => {
                                    match result {
                                        Ok(()) => break,
                                        Err(error) if is_temporary_transport(error.kind()) => {
                                            enter_reconnecting(
                                                local_writer,
                                                &state,
                                                &mut phase,
                                                operation_timeout,
                                            ).await?;
                                            continue 'bridge;
                                        }
                                        Err(error) => return Err(error),
                                    }
                                }
                                local = local_reader.next() => {
                                    match local? {
                                        Some(frame) => {
                                            if process_offline_local_frame(
                                                frame,
                                                &mut state,
                                                local_writer,
                                                operation_timeout,
                                            ).await? {
                                                return Ok(());
                                            }
                                        }
                                        None => return Ok(()),
                                    }
                                }
                            }
                        }
                    }

                    let required = loop {
                        tokio::select! {
                            remote = remote_reader.next_until(
                                attempt_deadline,
                                "remote full-sync marker exceeded its absolute deadline",
                            ) => {
                                match remote {
                                    Ok(Some(frame)) => break frame,
                                    Ok(None) => {
                                        enter_reconnecting(
                                            local_writer,
                                            &state,
                                            &mut phase,
                                            operation_timeout,
                                        ).await?;
                                        continue 'bridge;
                                    }
                                    Err(error) if is_temporary_transport(error.kind()) => {
                                        enter_reconnecting(
                                            local_writer,
                                            &state,
                                            &mut phase,
                                            operation_timeout,
                                        ).await?;
                                        continue 'bridge;
                                    }
                                    Err(error) => return Err(error),
                                }
                            }
                            local = local_reader.next() => {
                                match local? {
                                    Some(frame) => {
                                        if process_offline_local_frame(
                                            frame,
                                            &mut state,
                                            local_writer,
                                            operation_timeout,
                                        ).await? {
                                            let _ = send_remote_detach(
                                                &mut remote_writer,
                                                Some(remote_attachment_id),
                                                attempt_deadline,
                                            ).await;
                                            return Ok(());
                                        }
                                    }
                                    None => {
                                        let _ = send_remote_detach(
                                            &mut remote_writer,
                                            Some(remote_attachment_id),
                                            attempt_deadline,
                                        ).await;
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    };
                    let required_revision = accept_initial_sync_required(
                        required,
                        remote_attachment_id,
                        initial_request_id,
                    )?;

                    let snapshot = loop {
                        tokio::select! {
                            remote = remote_reader.next_until(
                                attempt_deadline,
                                "remote full-sync snapshot exceeded its absolute deadline",
                            ) => {
                                match remote {
                                    Ok(Some(frame)) => break frame,
                                    Ok(None) => {
                                        enter_reconnecting(
                                            local_writer,
                                            &state,
                                            &mut phase,
                                            operation_timeout,
                                        ).await?;
                                        continue 'bridge;
                                    }
                                    Err(error) if is_temporary_transport(error.kind()) => {
                                        enter_reconnecting(
                                            local_writer,
                                            &state,
                                            &mut phase,
                                            operation_timeout,
                                        ).await?;
                                        continue 'bridge;
                                    }
                                    Err(error) => return Err(error),
                                }
                            }
                            local = local_reader.next() => {
                                match local? {
                                    Some(frame) => {
                                        if process_offline_local_frame(
                                            frame,
                                            &mut state,
                                            local_writer,
                                            operation_timeout,
                                        ).await? {
                                            let _ = send_remote_detach(
                                                &mut remote_writer,
                                                Some(remote_attachment_id),
                                                attempt_deadline,
                                            ).await;
                                            return Ok(());
                                        }
                                    }
                                    None => {
                                        let _ = send_remote_detach(
                                            &mut remote_writer,
                                            Some(remote_attachment_id),
                                            attempt_deadline,
                                        ).await;
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    };
                    let InitialRemoteUpdate::Accepted(accepted) = accept_initial_remote_update(
                        snapshot,
                        &mut state,
                        Some(remote_attachment_id),
                        initial_request_id,
                    )?
                    else {
                        return Err(malformed(
                            "remote full synchronization returned another inconsistent delta",
                        ));
                    };
                    if accepted.revision != required_revision {
                        return Err(malformed(
                            "remote full-sync marker and snapshot revision mismatch",
                        ));
                    }
                    accepted
                }
            },
        };
        enter_synchronizing(local_writer, &state, &mut phase, attempt_deadline).await?;
        write_local(local_writer, &accepted.local_bytes, attempt_deadline).await?;

        let epoch = RemoteEpoch {
            reader: remote_reader,
            writer: remote_writer,
            observer,
            attachment_id: accepted.remote_attachment_id,
            initial_revision: accepted.revision,
        };
        let epoch_end = match run_epoch(
            epoch,
            local_reader,
            local_writer,
            &mut state,
            &mut phase,
            operation_timeout,
        )
        .await
        {
            Ok(end) => end,
            Err(error) => {
                resolve_pending_control(local_writer, &mut state, &error, operation_timeout)
                    .await?;
                return Err(error);
            }
        };
        match epoch_end {
            EpochEnd::Reconnect(error) => {
                resolve_pending_control(local_writer, &mut state, &error, operation_timeout)
                    .await?;
                enter_reconnecting(local_writer, &state, &mut phase, operation_timeout).await?;
            }
            EpochEnd::Terminal => {
                resolve_pending_control(
                    local_writer,
                    &mut state,
                    &transport_unavailable(
                        "remote attachment ended before its control responses arrived",
                    ),
                    operation_timeout,
                )
                .await?;
                return Ok(());
            }
            EpochEnd::Detached => return Ok(()),
        }
    }
}

struct AcceptedInitial {
    remote_attachment_id: AttachmentId,
    revision: Revision,
    local_bytes: Vec<u8>,
}

#[derive(Clone, Eq, PartialEq)]
enum EpochEnd {
    Reconnect(DaemonError),
    Detached,
    Terminal,
}

impl fmt::Debug for EpochEnd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reconnect(error) => formatter
                .debug_struct("Reconnect")
                .field("error_kind", &error.kind())
                .finish(),
            Self::Detached => formatter.write_str("Detached"),
            Self::Terminal => formatter.write_str("Terminal"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EpochPhase {
    Synchronizing {
        expected: Revision,
        acknowledged: bool,
        needs_takeover: bool,
    },
    Active,
}

struct EpochControl<'a> {
    phase: &'a mut EpochPhase,
    desired_phase: &'a mut DesiredViewPhase,
    operation_timeout: Duration,
}

async fn run_epoch<LocalReader, LocalWriter>(
    mut epoch: RemoteEpoch,
    local_reader: &mut FramedReader<LocalReader>,
    local_writer: &mut LocalWriter,
    state: &mut BridgeState,
    desired_phase: &mut DesiredViewPhase,
    operation_timeout: Duration,
) -> Result<EpochEnd, DaemonError>
where
    LocalReader: AsyncRead + Unpin,
    LocalWriter: AsyncWrite + Unpin,
{
    let history_paging = epoch.observer.supports(Capabilities::HISTORY_PAGING);
    let mut phase = EpochPhase::Synchronizing {
        expected: epoch.initial_revision,
        acknowledged: false,
        needs_takeover: state.request.takeover && !state.ever_active,
    };
    let mut path_observation = tokio::time::interval(Duration::from_secs(1));
    path_observation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_path_observation = None;
    loop {
        tokio::select! {
            local = local_reader.next() => {
                let Some(frame) = local? else {
                    let _ = send_remote_detach(
                        &mut epoch.writer,
                        Some(epoch.attachment_id),
                        Instant::now() + operation_timeout,
                    ).await;
                    return Ok(EpochEnd::Detached);
                };
                let request_id = frame.request_id;
                if frame.kind == WireKind::TerminalHistoryRequest && !history_paging {
                    if let Err(error) = write_unsupported_history_gap(
                        frame,
                        state,
                        local_writer,
                        Instant::now() + operation_timeout,
                    ).await {
                        write_error_best_effort(
                            local_writer,
                            request_id,
                            &error,
                            Instant::now() + operation_timeout,
                        ).await;
                        return Ok(EpochEnd::Terminal);
                    }
                    continue;
                }
                match process_epoch_local_frame(
                    frame,
                    &mut epoch.writer,
                    epoch.attachment_id,
                    state,
                    local_writer,
                    EpochControl {
                        phase: &mut phase,
                        desired_phase,
                        operation_timeout,
                    },
                ).await {
                    Ok(Some(end)) => return Ok(end),
                    Ok(None) => {}
                    Err(error) => {
                        write_error_best_effort(
                            local_writer,
                            request_id,
                            &error,
                            Instant::now() + operation_timeout,
                        ).await;
                        return Ok(EpochEnd::Terminal);
                    }
                }
            }
            remote = epoch.reader.next() => {
                let frame = match remote {
                    Ok(Some(frame)) => frame,
                    Ok(None) => return Ok(EpochEnd::Reconnect(transport_unavailable(
                        "remote terminal stream closed before the desired view detached",
                    ))),
                    Err(error) if is_temporary_transport(error.kind()) => {
                        return Ok(EpochEnd::Reconnect(error));
                    }
                    Err(error) => {
                        write_error_best_effort(
                            local_writer,
                            0,
                            &error,
                            Instant::now() + operation_timeout,
                        ).await;
                        return Ok(EpochEnd::Terminal);
                    }
                };
                let request_id = frame.request_id;
                match process_epoch_remote_frame(
                    frame,
                    epoch.attachment_id,
                    state,
                    local_writer,
                    &mut epoch.writer,
                    EpochControl {
                        phase: &mut phase,
                        desired_phase,
                        operation_timeout,
                    },
                ).await {
                    Ok(Some(end)) => return Ok(end),
                    Ok(None) => {}
                    Err(error) => {
                        write_error_best_effort(
                            local_writer,
                            request_id,
                            &error,
                            Instant::now() + operation_timeout,
                        ).await;
                        return Ok(EpochEnd::Terminal);
                    }
                }
            }
            _ = path_observation.tick(), if matches!(phase, EpochPhase::Active) => {
                let observation = epoch.observer.selected_path_observation();
                if last_path_observation != Some(observation) {
                    write_connection_status(
                        local_writer,
                        state.local_view_id,
                        observation,
                        Instant::now() + operation_timeout,
                    ).await?;
                    last_path_observation = Some(observation);
                }
            }
        }
    }
}

async fn enter_reconnecting<Writer>(
    writer: &mut Writer,
    state: &BridgeState,
    phase: &mut DesiredViewPhase,
    operation_timeout: Duration,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    if *phase != DesiredViewPhase::Reconnecting {
        *phase = DesiredViewPhase::Reconnecting;
        write_transport_state(
            writer,
            state.local_view_id,
            v1::TerminalTransportState::Reconnecting,
            Instant::now() + operation_timeout,
        )
        .await?;
        write_connection_status(
            writer,
            state.local_view_id,
            SelectedPathObservation::default(),
            Instant::now() + operation_timeout,
        )
        .await?;
    }
    Ok(())
}

async fn enter_synchronizing<Writer>(
    writer: &mut Writer,
    state: &BridgeState,
    phase: &mut DesiredViewPhase,
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    if *phase != DesiredViewPhase::Synchronizing {
        write_transport_state(
            writer,
            state.local_view_id,
            v1::TerminalTransportState::Synchronizing,
            deadline,
        )
        .await?;
        *phase = DesiredViewPhase::Synchronizing;
    }
    Ok(())
}

fn should_retry_resume_occupied(state: &BridgeState, error: &DaemonError) -> bool {
    state.ever_active
        && state.frozen_session_id.is_some()
        && error.kind() == DomainErrorKind::SessionOccupied
}

async fn wait_before_resume_occupied_retry<LocalReader, LocalWriter>(
    local_reader: &mut FramedReader<LocalReader>,
    local_writer: &mut LocalWriter,
    state: &mut BridgeState,
    operation_timeout: Duration,
) -> Result<bool, DaemonError>
where
    LocalReader: AsyncRead + Unpin,
    LocalWriter: AsyncWrite + Unpin,
{
    let delay = tokio::time::sleep(RESUME_OCCUPIED_RETRY_DELAY);
    tokio::pin!(delay);
    loop {
        tokio::select! {
            biased;
            () = &mut delay => return Ok(false),
            local = local_reader.next() => {
                match local? {
                    Some(frame) => {
                        if process_offline_local_frame(
                            frame,
                            state,
                            local_writer,
                            operation_timeout,
                        ).await? {
                            return Ok(true);
                        }
                    }
                    None => return Ok(true),
                }
            }
        }
    }
}

async fn process_offline_local_frame<Writer>(
    frame: DecodedFrame,
    state: &mut BridgeState,
    local_writer: &mut Writer,
    operation_timeout: Duration,
) -> Result<bool, DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    let request_id = frame.request_id;
    match frame.kind {
        WireKind::TerminalDetach => {
            let request: v1::TerminalDetach = decode(&frame)?;
            require_local_attachment(request.attachment_id, state.local_view_id)?;
            Ok(true)
        }
        WireKind::TerminalResize => {
            let request: v1::TerminalResize = decode(&frame)?;
            require_local_attachment(request.attachment_id, state.local_view_id)?;
            let viewport = v1::TerminalViewport {
                rows: request.rows,
                columns: request.columns,
            };
            zterm_core::terminal::TerminalSize::try_from(viewport).map_err(protocol_error)?;
            state.latest_viewport = Some(viewport);
            Ok(false)
        }
        WireKind::TerminalInput => {
            let request: v1::TerminalInput = decode(&frame)?;
            require_local_attachment(request.attachment_id, state.local_view_id)?;
            Ok(false)
        }
        WireKind::TerminalSnapshotApplied => {
            let request: v1::TerminalSnapshotApplied = decode(&frame)?;
            require_local_attachment(request.attachment_id, state.local_view_id)?;
            Ok(false)
        }
        WireKind::TerminalSyncRequest => {
            let request: v1::TerminalSyncRequest = decode(&frame)?;
            require_local_attachment(request.attachment_id, state.local_view_id)?;
            state.force_full = true;
            Ok(false)
        }
        WireKind::TerminalHistoryRequest => {
            let request: v1::TerminalHistoryRequest = decode(&frame)?;
            require_local_attachment(request.attachment_id.clone(), state.local_view_id)?;
            validate_history_request(&request)?;
            write_service_error(
                local_writer,
                request_id,
                &transport_unavailable("remote attachment is not active"),
                Instant::now() + operation_timeout,
            )
            .await?;
            Ok(false)
        }
        WireKind::SessionTakeoverRequest => {
            let request: v1::SessionTakeoverRequest = decode(&frame)?;
            require_local_attachment(request.attachment_id, state.local_view_id)?;
            require_frozen_session(request.session_id, state.frozen_session_id)?;
            require_exact_target(request.target, state.target)?;
            write_service_error(
                local_writer,
                request_id,
                &transport_unavailable("remote attachment is not active"),
                Instant::now() + operation_timeout,
            )
            .await?;
            Ok(false)
        }
        WireKind::SessionOperationLeaseRequest => {
            let request: v1::SessionOperationLeaseRequest = decode(&frame)?;
            require_exact_target(request.target, state.target)?;
            write_service_error(
                local_writer,
                request_id,
                &transport_unavailable("remote attachment is not active"),
                Instant::now() + operation_timeout,
            )
            .await?;
            Ok(false)
        }
        _ => Err(malformed(format!(
            "wire kind {:?} is invalid from a local remote attachment",
            frame.kind
        ))),
    }
}

fn remote_attach_request(state: &BridgeState) -> v1::TerminalAttachRequest {
    let mut request = state.request.clone();
    request.target = Some(v1::TargetSelector {
        target: Some(v1::target_selector::Target::Device(state.target.into())),
    });
    request.resume_view_id = Some(state.resume_view_id.into());
    request.known_revision = (!state.force_full)
        .then_some(state.known_revision)
        .flatten()
        .map(Revision::get);
    if let Some(session_id) = state.frozen_session_id {
        request.viewport = None;
        request.session_id = Some(session_id.into());
        request.session_name.clear();
        request.create_main = false;
        request.takeover = state.request.takeover && !state.ever_active;
    } else {
        request.viewport = state.latest_viewport;
    }
    request
}

enum InitialRemoteUpdate {
    Accepted(AcceptedInitial),
    RequiresFull {
        remote_attachment_id: AttachmentId,
        known_revision: Revision,
    },
}

fn accept_initial_remote_update(
    frame: DecodedFrame,
    state: &mut BridgeState,
    expected_remote_attachment_id: Option<AttachmentId>,
    expected_request_id: u64,
) -> Result<InitialRemoteUpdate, DaemonError> {
    if frame.request_id != expected_request_id {
        return Err(malformed(
            "remote initial terminal update request_id mismatch",
        ));
    }
    if frame.kind == WireKind::ServiceErrorResponse {
        return Err(decode_session_service_error(&frame)?);
    }
    match frame.kind {
        WireKind::TerminalSnapshot => {
            let mut snapshot: v1::TerminalSnapshot = decode(&frame)?;
            validate_snapshot(&snapshot)?;
            let session_id = required_session_id(snapshot.session_id.clone())?;
            freeze_session(state, session_id)?;
            let remote_attachment_id = required_attachment_id(snapshot.attachment_id.clone())?;
            require_expected_remote_attachment(
                remote_attachment_id,
                expected_remote_attachment_id,
            )?;
            snapshot.attachment_id = Some(state.local_view_id.into());
            let revision = Revision::new(snapshot.revision);
            let local_bytes = encode_message(
                WireKind::TerminalSnapshot,
                frame.request_id,
                frame.deadline_ms,
                &snapshot,
            )
            .map_err(protocol_error)?;
            Ok(InitialRemoteUpdate::Accepted(AcceptedInitial {
                remote_attachment_id,
                revision,
                local_bytes,
            }))
        }
        WireKind::TerminalDelta => {
            let mut delta: v1::TerminalDelta = decode(&frame)?;
            validate_delta(&delta)?;
            let remote_attachment_id = required_attachment_id(delta.attachment_id.clone())?;
            require_expected_remote_attachment(
                remote_attachment_id,
                expected_remote_attachment_id,
            )?;
            let known = state
                .known_revision
                .ok_or_else(|| malformed("an initial resume delta has no local baseline"))?;
            if state.frozen_session_id.is_none()
                || delta.from_revision != known.get()
                || delta.to_revision < delta.from_revision
            {
                return Ok(InitialRemoteUpdate::RequiresFull {
                    remote_attachment_id,
                    known_revision: known,
                });
            }
            delta.attachment_id = Some(state.local_view_id.into());
            let revision = Revision::new(delta.to_revision);
            let local_bytes = encode_message(
                WireKind::TerminalDelta,
                frame.request_id,
                frame.deadline_ms,
                &delta,
            )
            .map_err(protocol_error)?;
            Ok(InitialRemoteUpdate::Accepted(AcceptedInitial {
                remote_attachment_id,
                revision,
                local_bytes,
            }))
        }
        _ => Err(malformed(format!(
            "remote attach began with unexpected wire kind {:?}",
            frame.kind
        ))),
    }
}

fn accept_initial_sync_required(
    frame: DecodedFrame,
    expected_attachment_id: AttachmentId,
    expected_request_id: u64,
) -> Result<Revision, DaemonError> {
    if frame.request_id != expected_request_id {
        return Err(malformed("remote full-sync marker request_id mismatch"));
    }
    if frame.kind == WireKind::ServiceErrorResponse {
        return Err(decode_session_service_error(&frame)?);
    }
    if frame.kind != WireKind::TerminalSyncRequired {
        return Err(malformed(format!(
            "remote full synchronization expected TerminalSyncRequired, got {:?}",
            frame.kind
        )));
    }
    let required: v1::TerminalSyncRequired = decode(&frame)?;
    require_remote_attachment(required.attachment_id, expected_attachment_id)?;
    Ok(Revision::new(required.latest_revision))
}

async fn process_epoch_local_frame<RemoteWriter, LocalWriter>(
    frame: DecodedFrame,
    remote_writer: &mut RemoteWriter,
    remote_attachment_id: AttachmentId,
    state: &mut BridgeState,
    local_writer: &mut LocalWriter,
    control: EpochControl<'_>,
) -> Result<Option<EpochEnd>, DaemonError>
where
    RemoteWriter: AsyncWrite + Unpin,
    LocalWriter: AsyncWrite + Unpin,
{
    let EpochControl {
        phase,
        desired_phase,
        operation_timeout,
    } = control;
    match frame.kind {
        WireKind::TerminalDetach => {
            let request: v1::TerminalDetach = decode(&frame)?;
            require_local_attachment(request.attachment_id, state.local_view_id)?;
            let bytes = encode_message(
                WireKind::TerminalDetach,
                frame.request_id,
                frame.deadline_ms,
                &v1::TerminalDetach {
                    attachment_id: Some(remote_attachment_id.into()),
                },
            )
            .map_err(protocol_error)?;
            let deadline = Instant::now() + operation_timeout;
            let _ = write_remote(remote_writer, &bytes, deadline).await;
            let _ = timeout_at(
                deadline,
                remote_writer.shutdown(),
                "remote terminal detach exceeded its absolute deadline",
            )
            .await;
            Ok(Some(EpochEnd::Detached))
        }
        WireKind::TerminalInput => {
            let mut request: v1::TerminalInput = decode(&frame)?;
            require_local_attachment(request.attachment_id.clone(), state.local_view_id)?;
            if !matches!(phase, EpochPhase::Active) {
                return Ok(None);
            }
            request.attachment_id = Some(remote_attachment_id.into());
            if let Err(error) = forward_remote(
                frame,
                request,
                remote_writer,
                Instant::now() + operation_timeout,
            )
            .await
            {
                Ok(Some(EpochEnd::Reconnect(error)))
            } else {
                Ok(None)
            }
        }
        WireKind::TerminalResize => {
            let mut request: v1::TerminalResize = decode(&frame)?;
            require_local_attachment(request.attachment_id.clone(), state.local_view_id)?;
            let viewport = v1::TerminalViewport {
                rows: request.rows,
                columns: request.columns,
            };
            zterm_core::terminal::TerminalSize::try_from(viewport).map_err(protocol_error)?;
            state.latest_viewport = Some(viewport);
            if !matches!(phase, EpochPhase::Active) {
                return Ok(None);
            }
            request.attachment_id = Some(remote_attachment_id.into());
            if let Err(error) = forward_remote(
                frame,
                request,
                remote_writer,
                Instant::now() + operation_timeout,
            )
            .await
            {
                Ok(Some(EpochEnd::Reconnect(error)))
            } else {
                Ok(None)
            }
        }
        WireKind::TerminalSnapshotApplied => {
            let mut request: v1::TerminalSnapshotApplied = decode(&frame)?;
            require_local_attachment(request.attachment_id.clone(), state.local_view_id)?;
            let EpochPhase::Synchronizing {
                expected,
                acknowledged,
                needs_takeover,
            } = phase
            else {
                return Ok(None);
            };
            if *acknowledged || request.revision != expected.get() {
                return Ok(None);
            }
            state.known_revision = Some(*expected);
            request.attachment_id = Some(remote_attachment_id.into());
            if let Err(error) = forward_remote(
                frame,
                request,
                remote_writer,
                Instant::now() + operation_timeout,
            )
            .await
            {
                return Ok(Some(EpochEnd::Reconnect(error)));
            }
            *acknowledged = true;
            if !*needs_takeover
                && let Some(error) = activate_epoch(
                    remote_writer,
                    remote_attachment_id,
                    state,
                    phase,
                    local_writer,
                    desired_phase,
                    operation_timeout,
                )
                .await?
            {
                return Ok(Some(EpochEnd::Reconnect(error)));
            }
            Ok(None)
        }
        WireKind::TerminalSyncRequest => {
            let mut request: v1::TerminalSyncRequest = decode(&frame)?;
            require_local_attachment(request.attachment_id.clone(), state.local_view_id)?;
            if !matches!(phase, EpochPhase::Active) {
                state.force_full = true;
                return Ok(None);
            }
            request.attachment_id = Some(remote_attachment_id.into());
            if let Err(error) = forward_remote(
                frame,
                request,
                remote_writer,
                Instant::now() + operation_timeout,
            )
            .await
            {
                return Ok(Some(EpochEnd::Reconnect(error)));
            }
            let expected = state.known_revision.unwrap_or(Revision::ZERO);
            *phase = EpochPhase::Synchronizing {
                expected,
                acknowledged: true,
                needs_takeover: false,
            };
            enter_synchronizing(
                local_writer,
                state,
                desired_phase,
                Instant::now() + operation_timeout,
            )
            .await?;
            Ok(None)
        }
        WireKind::TerminalHistoryRequest => {
            let mut request: v1::TerminalHistoryRequest = decode(&frame)?;
            require_local_attachment(request.attachment_id.clone(), state.local_view_id)?;
            validate_history_request(&request)?;
            if !matches!(phase, EpochPhase::Active) {
                write_service_error(
                    local_writer,
                    frame.request_id,
                    &DaemonError::new(
                        DomainErrorKind::NotSynchronized,
                        "history paging requires an active remote attachment",
                    ),
                    Instant::now() + operation_timeout,
                )
                .await?;
                return Ok(None);
            }
            request.attachment_id = Some(remote_attachment_id.into());
            retain_pending(
                &mut state.pending_control,
                frame.request_id,
                WireKind::TerminalHistoryPage,
            )?;
            if let Err(error) = forward_remote(
                frame,
                request,
                remote_writer,
                Instant::now() + operation_timeout,
            )
            .await
            {
                Ok(Some(EpochEnd::Reconnect(error)))
            } else {
                Ok(None)
            }
        }
        WireKind::SessionOperationLeaseRequest => {
            let request: v1::SessionOperationLeaseRequest = decode(&frame)?;
            require_exact_target(request.target.clone(), state.target)?;
            let allowed = matches!(phase, EpochPhase::Active)
                || matches!(
                    phase,
                    EpochPhase::Synchronizing {
                        acknowledged: true,
                        needs_takeover: true,
                        ..
                    }
                );
            if !allowed {
                write_service_error(
                    local_writer,
                    frame.request_id,
                    &DaemonError::new(
                        DomainErrorKind::NotSynchronized,
                        "operation lease allocation requires an active remote attachment",
                    ),
                    Instant::now() + operation_timeout,
                )
                .await?;
                return Ok(None);
            }
            retain_pending(
                &mut state.pending_control,
                frame.request_id,
                WireKind::SessionOperationLeaseResponse,
            )?;
            if let Err(error) = forward_remote(
                frame,
                request,
                remote_writer,
                Instant::now() + operation_timeout,
            )
            .await
            {
                Ok(Some(EpochEnd::Reconnect(error)))
            } else {
                Ok(None)
            }
        }
        WireKind::SessionTakeoverRequest => {
            let mut request: v1::SessionTakeoverRequest = decode(&frame)?;
            require_local_attachment(request.attachment_id.clone(), state.local_view_id)?;
            require_frozen_session(request.session_id.clone(), state.frozen_session_id)?;
            require_exact_target(request.target.clone(), state.target)?;
            let allowed = matches!(phase, EpochPhase::Active)
                || matches!(
                    phase,
                    EpochPhase::Synchronizing {
                        acknowledged: true,
                        needs_takeover: true,
                        ..
                    }
                );
            if !allowed {
                write_service_error(
                    local_writer,
                    frame.request_id,
                    &DaemonError::new(
                        DomainErrorKind::NotSynchronized,
                        "takeover requires an acknowledged remote attachment",
                    ),
                    Instant::now() + operation_timeout,
                )
                .await?;
                return Ok(None);
            }
            request.attachment_id = Some(remote_attachment_id.into());
            retain_pending(
                &mut state.pending_control,
                frame.request_id,
                WireKind::SessionMutateResponse,
            )?;
            if let Err(error) = forward_remote(
                frame,
                request,
                remote_writer,
                Instant::now() + operation_timeout,
            )
            .await
            {
                Ok(Some(EpochEnd::Reconnect(error)))
            } else {
                Ok(None)
            }
        }
        _ => Err(malformed(format!(
            "wire kind {:?} is invalid from a local remote attachment",
            frame.kind
        ))),
    }
}

async fn process_epoch_remote_frame<LocalWriter, RemoteWriter>(
    frame: DecodedFrame,
    remote_attachment_id: AttachmentId,
    state: &mut BridgeState,
    local_writer: &mut LocalWriter,
    remote_writer: &mut RemoteWriter,
    control: EpochControl<'_>,
) -> Result<Option<EpochEnd>, DaemonError>
where
    LocalWriter: AsyncWrite + Unpin,
    RemoteWriter: AsyncWrite + Unpin,
{
    let EpochControl {
        phase,
        desired_phase,
        operation_timeout,
    } = control;
    if frame.kind == WireKind::ServiceErrorResponse {
        let error = decode_session_service_error(&frame)?;
        let correlated = if let Some(expected) = state.pending_control.get(&frame.request_id) {
            let status = validate_session_unary_response(&frame, frame.request_id, *expected)?;
            if !matches!(status, SessionUnaryResponseStatus::ServiceError(_)) {
                return Err(malformed(
                    "remote attachment service error failed response validation",
                ));
            }
            state.pending_control.remove(&frame.request_id);
            true
        } else {
            false
        };
        let bytes = ServiceReply::error(frame.request_id, &error).bytes;
        write_local(
            local_writer,
            bytes.as_ref(),
            Instant::now() + operation_timeout,
        )
        .await?;
        return if correlated && !is_fatal_attachment_service_error(error.kind()) {
            Ok(None)
        } else {
            Ok(Some(EpochEnd::Terminal))
        };
    }
    match frame.kind {
        WireKind::TerminalDelta => {
            let mut delta: v1::TerminalDelta = decode(&frame)?;
            validate_delta(&delta)?;
            require_remote_attachment(delta.attachment_id.clone(), remote_attachment_id)?;
            if !matches!(phase, EpochPhase::Active) {
                return Err(malformed(
                    "remote terminal delta arrived before initial synchronization",
                ));
            }
            let known = state.known_revision.unwrap_or(Revision::ZERO);
            if delta.from_revision != known.get() || delta.to_revision < delta.from_revision {
                let request = v1::TerminalSyncRequest {
                    attachment_id: Some(remote_attachment_id.into()),
                    known_revision: known.get(),
                };
                let bytes = encode_message(WireKind::TerminalSyncRequest, 0, 0, &request)
                    .map_err(protocol_error)?;
                if let Err(error) =
                    write_remote(remote_writer, &bytes, Instant::now() + operation_timeout).await
                {
                    return Ok(Some(EpochEnd::Reconnect(error)));
                }
                state.force_full = true;
                *phase = EpochPhase::Synchronizing {
                    expected: known,
                    acknowledged: true,
                    needs_takeover: false,
                };
                enter_synchronizing(
                    local_writer,
                    state,
                    desired_phase,
                    Instant::now() + operation_timeout,
                )
                .await?;
                return Ok(None);
            }
            delta.attachment_id = Some(state.local_view_id.into());
            let to_revision = Revision::new(delta.to_revision);
            let bytes = encode_message(
                WireKind::TerminalDelta,
                frame.request_id,
                frame.deadline_ms,
                &delta,
            )
            .map_err(protocol_error)?;
            write_local(local_writer, &bytes, Instant::now() + operation_timeout).await?;
            state.known_revision = Some(to_revision);
            Ok(None)
        }
        WireKind::TerminalSyncRequired => {
            let mut required: v1::TerminalSyncRequired = decode(&frame)?;
            require_remote_attachment(required.attachment_id.clone(), remote_attachment_id)?;
            required.attachment_id = Some(state.local_view_id.into());
            let expected = Revision::new(required.latest_revision);
            let needs_takeover = matches!(
                phase,
                EpochPhase::Synchronizing {
                    needs_takeover: true,
                    ..
                }
            );
            *phase = EpochPhase::Synchronizing {
                expected,
                acknowledged: false,
                needs_takeover,
            };
            let bytes = encode_message(
                WireKind::TerminalSyncRequired,
                frame.request_id,
                frame.deadline_ms,
                &required,
            )
            .map_err(protocol_error)?;
            enter_synchronizing(
                local_writer,
                state,
                desired_phase,
                Instant::now() + operation_timeout,
            )
            .await?;
            write_local(local_writer, &bytes, Instant::now() + operation_timeout).await?;
            Ok(None)
        }
        WireKind::TerminalSnapshot => {
            let mut snapshot: v1::TerminalSnapshot = decode(&frame)?;
            validate_snapshot(&snapshot)?;
            require_remote_attachment(snapshot.attachment_id.clone(), remote_attachment_id)?;
            let session_id = required_session_id(snapshot.session_id.clone())?;
            freeze_session(state, session_id)?;
            snapshot.attachment_id = Some(state.local_view_id.into());
            let expected = Revision::new(snapshot.revision);
            let needs_takeover = matches!(
                phase,
                EpochPhase::Synchronizing {
                    needs_takeover: true,
                    ..
                }
            );
            *phase = EpochPhase::Synchronizing {
                expected,
                acknowledged: false,
                needs_takeover,
            };
            state.force_full = false;
            let bytes = encode_message(
                WireKind::TerminalSnapshot,
                frame.request_id,
                frame.deadline_ms,
                &snapshot,
            )
            .map_err(protocol_error)?;
            enter_synchronizing(
                local_writer,
                state,
                desired_phase,
                Instant::now() + operation_timeout,
            )
            .await?;
            write_local(local_writer, &bytes, Instant::now() + operation_timeout).await?;
            Ok(None)
        }
        WireKind::TerminalLeaseLost => {
            let mut lost: v1::TerminalLeaseLost = decode(&frame)?;
            require_remote_attachment(lost.attachment_id.clone(), remote_attachment_id)?;
            lost.attachment_id = Some(state.local_view_id.into());
            let bytes = encode_message(
                WireKind::TerminalLeaseLost,
                frame.request_id,
                frame.deadline_ms,
                &lost,
            )
            .map_err(protocol_error)?;
            write_local(local_writer, &bytes, Instant::now() + operation_timeout).await?;
            Ok(Some(EpochEnd::Terminal))
        }
        WireKind::TerminalSessionEnded => {
            let mut ended: v1::TerminalSessionEnded = decode(&frame)?;
            require_remote_attachment(ended.attachment_id.clone(), remote_attachment_id)?;
            require_frozen_session(ended.session_id.clone(), state.frozen_session_id)?;
            ended.attachment_id = Some(state.local_view_id.into());
            let bytes = encode_message(
                WireKind::TerminalSessionEnded,
                frame.request_id,
                frame.deadline_ms,
                &ended,
            )
            .map_err(protocol_error)?;
            write_local(local_writer, &bytes, Instant::now() + operation_timeout).await?;
            Ok(Some(EpochEnd::Terminal))
        }
        WireKind::TerminalHistoryPage => {
            let expected = state
                .pending_control
                .get(&frame.request_id)
                .copied()
                .ok_or_else(|| malformed("unsolicited remote terminal history page"))?;
            if expected != WireKind::TerminalHistoryPage {
                return Err(malformed(
                    "remote attachment control response kind mismatch",
                ));
            }
            let mut page: v1::TerminalHistoryPage = decode(&frame)?;
            require_remote_attachment(page.attachment_id.clone(), remote_attachment_id)?;
            validate_history_page(&page)?;
            page.attachment_id = Some(state.local_view_id.into());
            let bytes = encode_message(
                WireKind::TerminalHistoryPage,
                frame.request_id,
                frame.deadline_ms,
                &page,
            )
            .map_err(protocol_error)?;
            write_local(local_writer, &bytes, Instant::now() + operation_timeout).await?;
            state.pending_control.remove(&frame.request_id);
            Ok(None)
        }
        WireKind::SessionOperationLeaseResponse | WireKind::SessionMutateResponse => {
            let expected = state
                .pending_control
                .get(&frame.request_id)
                .copied()
                .ok_or_else(|| malformed("unsolicited remote attachment control response"))?;
            if expected != frame.kind {
                return Err(malformed(
                    "remote attachment control response kind mismatch",
                ));
            }
            validate_session_unary_response(&frame, frame.request_id, expected)?;
            state.pending_control.remove(&frame.request_id);
            let bytes = match frame.kind {
                WireKind::SessionOperationLeaseResponse => {
                    let response: v1::SessionOperationLeaseResponse = decode(&frame)?;
                    encode_message(frame.kind, frame.request_id, frame.deadline_ms, &response)
                        .map_err(protocol_error)?
                }
                WireKind::SessionMutateResponse => {
                    let response: v1::SessionMutateResponse = decode(&frame)?;
                    let summary = response
                        .session
                        .clone()
                        .ok_or_else(|| malformed("takeover response omitted session"))?;
                    let summary = session_summary_from_wire(summary)?;
                    if Some(summary.session_id) != state.frozen_session_id {
                        return Err(malformed("takeover response changed the frozen session"));
                    }
                    encode_message(frame.kind, frame.request_id, frame.deadline_ms, &response)
                        .map_err(protocol_error)?
                }
                _ => unreachable!(),
            };
            write_local(local_writer, &bytes, Instant::now() + operation_timeout).await?;
            let takeover_completed = frame.kind == WireKind::SessionMutateResponse
                && matches!(
                    phase,
                    EpochPhase::Synchronizing {
                        acknowledged: true,
                        needs_takeover: true,
                        ..
                    }
                );
            if takeover_completed
                && let Some(error) = activate_epoch(
                    remote_writer,
                    remote_attachment_id,
                    state,
                    phase,
                    local_writer,
                    desired_phase,
                    operation_timeout,
                )
                .await?
            {
                return Ok(Some(EpochEnd::Reconnect(error)));
            }
            Ok(None)
        }
        _ => Err(malformed(format!(
            "wire kind {:?} is invalid from a remote terminal attachment",
            frame.kind
        ))),
    }
}

async fn activate_epoch<RemoteWriter, LocalWriter>(
    remote_writer: &mut RemoteWriter,
    remote_attachment_id: AttachmentId,
    state: &mut BridgeState,
    phase: &mut EpochPhase,
    local_writer: &mut LocalWriter,
    desired_phase: &mut DesiredViewPhase,
    operation_timeout: Duration,
) -> Result<Option<DaemonError>, DaemonError>
where
    RemoteWriter: AsyncWrite + Unpin,
    LocalWriter: AsyncWrite + Unpin,
{
    if let Some(viewport) = state.latest_viewport {
        let resize = v1::TerminalResize {
            operation_id: None,
            attachment_id: Some(remote_attachment_id.into()),
            rows: viewport.rows,
            columns: viewport.columns,
        };
        let bytes =
            encode_message(WireKind::TerminalResize, 0, 0, &resize).map_err(protocol_error)?;
        if let Err(error) =
            write_remote(remote_writer, &bytes, Instant::now() + operation_timeout).await
        {
            return Ok(Some(error));
        }
    }
    *phase = EpochPhase::Active;
    *desired_phase = DesiredViewPhase::Active;
    state.ever_active = true;
    state.request.takeover = false;
    state.force_full = false;
    write_transport_state(
        local_writer,
        state.local_view_id,
        v1::TerminalTransportState::Active,
        Instant::now() + operation_timeout,
    )
    .await?;
    Ok(None)
}

async fn forward_remote<Message, Writer>(
    frame: DecodedFrame,
    message: Message,
    writer: &mut Writer,
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Message: prost::Message,
    Writer: AsyncWrite + Unpin,
{
    let bytes = encode_message(frame.kind, frame.request_id, frame.deadline_ms, &message)
        .map_err(protocol_error)?;
    write_remote(writer, &bytes, deadline).await
}

fn retain_pending(
    pending: &mut BTreeMap<u64, WireKind>,
    request_id: u64,
    response_kind: WireKind,
) -> Result<(), DaemonError> {
    if request_id == 0 || pending.contains_key(&request_id) {
        return Err(malformed(
            "attachment control request_id must be unique and non-zero",
        ));
    }
    if pending.len() >= MAX_PENDING_CONTROL_REQUESTS {
        return Err(DaemonError::new(
            DomainErrorKind::ResourceExhausted,
            "attachment control response window is full",
        ));
    }
    pending.insert(request_id, response_kind);
    Ok(())
}

async fn resolve_pending_control<Writer>(
    writer: &mut Writer,
    state: &mut BridgeState,
    transport_error: &DaemonError,
    operation_timeout: Duration,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    let pending = std::mem::take(&mut state.pending_control);
    let deadline = Instant::now() + operation_timeout;
    for (request_id, response_kind) in pending {
        let error = match response_kind {
            WireKind::SessionOperationLeaseResponse | WireKind::TerminalHistoryPage => {
                transport_error.clone()
            }
            WireKind::SessionMutateResponse => DaemonError::new(
                DomainErrorKind::OperationOutcomeUnknown,
                "remote takeover result was lost with its stream epoch",
            ),
            _ => {
                return Err(malformed(
                    "remote attachment retained an unsupported control response kind",
                ));
            }
        };
        write_service_error(writer, request_id, &error, deadline).await?;
    }
    Ok(())
}

async fn send_remote_detach<Writer>(
    writer: &mut Writer,
    remote_attachment_id: Option<AttachmentId>,
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    let Some(remote_attachment_id) = remote_attachment_id else {
        return Ok(());
    };
    let bytes = encode_message(
        WireKind::TerminalDetach,
        0,
        0,
        &v1::TerminalDetach {
            attachment_id: Some(remote_attachment_id.into()),
        },
    )
    .map_err(protocol_error)?;
    let result = write_remote(writer, &bytes, deadline).await;
    if result.is_ok() {
        let _ = timeout_at(
            deadline,
            writer.shutdown(),
            "remote terminal detach finish exceeded its absolute deadline",
        )
        .await;
    }
    result
}

fn is_fatal_attachment_service_error(kind: DomainErrorKind) -> bool {
    matches!(
        kind,
        DomainErrorKind::Unauthorized
            | DomainErrorKind::AuthorizationRevoked
            | DomainErrorKind::WireMajorMismatch
            | DomainErrorKind::UnknownKind
            | DomainErrorKind::FrameTooLarge
            | DomainErrorKind::ControlPayloadTooLarge
            | DomainErrorKind::MalformedFrame
            | DomainErrorKind::ServiceNotImplemented
            | DomainErrorKind::SessionNotFound
            | DomainErrorKind::LeaseLost
    )
}

fn require_expected_remote_attachment(
    actual: AttachmentId,
    expected: Option<AttachmentId>,
) -> Result<(), DaemonError> {
    if expected.is_none_or(|expected| expected == actual) {
        Ok(())
    } else {
        Err(malformed(
            "remote full synchronization changed the current stream attachment_id",
        ))
    }
}

fn require_initial_target(
    request: &v1::TerminalAttachRequest,
    expected: DeviceId,
) -> Result<(), DaemonError> {
    require_exact_target(request.target.clone(), expected)
}

fn require_exact_target(
    target: Option<v1::TargetSelector>,
    expected: DeviceId,
) -> Result<(), DaemonError> {
    let Some(v1::target_selector::Target::Device(device)) = target.and_then(|target| target.target)
    else {
        return Err(malformed(
            "remote attachment requires one frozen full device target",
        ));
    };
    let actual: DeviceId = device.try_into().map_err(protocol_error)?;
    if actual == expected {
        Ok(())
    } else {
        Err(malformed(
            "remote attachment target differs from its routed device",
        ))
    }
}

fn require_local_attachment(
    attachment_id: Option<v1::AttachmentId>,
    expected: AttachmentId,
) -> Result<(), DaemonError> {
    let actual = required_attachment_id(attachment_id)?;
    if actual == expected {
        Ok(())
    } else {
        Err(malformed(
            "local terminal message attachment_id does not match this view",
        ))
    }
}

fn require_remote_attachment(
    attachment_id: Option<v1::AttachmentId>,
    expected: AttachmentId,
) -> Result<(), DaemonError> {
    let actual = required_attachment_id(attachment_id)?;
    if actual == expected {
        Ok(())
    } else {
        Err(malformed(
            "remote terminal message attachment_id does not match this stream epoch",
        ))
    }
}

fn required_attachment_id(
    attachment_id: Option<v1::AttachmentId>,
) -> Result<AttachmentId, DaemonError> {
    attachment_id
        .ok_or_else(|| malformed("terminal message omitted attachment_id"))?
        .try_into()
        .map_err(protocol_error)
}

fn required_session_id(session_id: Option<v1::SessionId>) -> Result<SessionId, DaemonError> {
    session_id
        .ok_or_else(|| malformed("terminal message omitted session_id"))?
        .try_into()
        .map_err(protocol_error)
}

fn require_frozen_session(
    session_id: Option<v1::SessionId>,
    frozen: Option<SessionId>,
) -> Result<(), DaemonError> {
    let actual = required_session_id(session_id)?;
    if Some(actual) == frozen {
        Ok(())
    } else {
        Err(malformed(
            "terminal message session_id does not match the frozen session",
        ))
    }
}

fn freeze_session(state: &mut BridgeState, session_id: SessionId) -> Result<(), DaemonError> {
    match state.frozen_session_id {
        Some(frozen) if frozen != session_id => Err(DaemonError::new(
            DomainErrorKind::SessionNotFound,
            "remote reconnect resolved a different daemon-lifetime session",
        )),
        Some(_) => Ok(()),
        None => {
            state.frozen_session_id = Some(session_id);
            Ok(())
        }
    }
}

fn validate_snapshot(snapshot: &v1::TerminalSnapshot) -> Result<(), DaemonError> {
    let viewport = v1::TerminalViewport {
        rows: snapshot.rows,
        columns: snapshot.columns,
    };
    zterm_core::terminal::TerminalSize::try_from(viewport).map_err(protocol_error)?;
    v1::TerminalActiveScreen::try_from(snapshot.active_screen)
        .map_err(|_| malformed("terminal snapshot used an unknown active screen"))?;
    if snapshot.modes.is_none() {
        return Err(malformed("terminal snapshot omitted modes"));
    }
    Ok(())
}

fn validate_delta(delta: &v1::TerminalDelta) -> Result<(), DaemonError> {
    let viewport = v1::TerminalViewport {
        rows: delta.rows,
        columns: delta.columns,
    };
    zterm_core::terminal::TerminalSize::try_from(viewport).map_err(protocol_error)?;
    v1::TerminalActiveScreen::try_from(delta.active_screen)
        .map_err(|_| malformed("terminal delta used an unknown active screen"))?;
    if delta.modes.is_none() {
        return Err(malformed("terminal delta omitted modes"));
    }
    Ok(())
}

fn validate_history_request(request: &v1::TerminalHistoryRequest) -> Result<(), DaemonError> {
    match v1::TerminalHistoryDirection::try_from(request.direction) {
        Ok(v1::TerminalHistoryDirection::Newest)
        | Ok(v1::TerminalHistoryDirection::Older)
        | Ok(v1::TerminalHistoryDirection::Newer) => {}
        Ok(v1::TerminalHistoryDirection::Unspecified) | Err(_) => {
            return Err(malformed("terminal history direction is invalid"));
        }
    }
    let maximum_rows = usize::try_from(request.maximum_rows)
        .map_err(|_| malformed("terminal history page bound is not representable"))?;
    if maximum_rows == 0 || maximum_rows > MAX_HISTORY_PAGE_ROWS {
        return Err(malformed(
            "terminal history page bound is outside the allowed range",
        ));
    }
    Ok(())
}

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

fn decode<Message>(frame: &DecodedFrame) -> Result<Message, DaemonError>
where
    Message: prost::Message + Default,
{
    frame.decode_message(frame.kind).map_err(protocol_error)
}

async fn write_transport_state<Writer>(
    writer: &mut Writer,
    attachment_id: AttachmentId,
    state: v1::TerminalTransportState,
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    let message = v1::TerminalTransportStateEvent {
        attachment_id: Some(attachment_id.into()),
        state: state as i32,
    };
    let bytes = encode_message(WireKind::TerminalTransportStateEvent, 0, 0, &message)
        .map_err(protocol_error)?;
    write_local(writer, &bytes, deadline).await
}

async fn write_connection_status<Writer>(
    writer: &mut Writer,
    attachment_id: AttachmentId,
    observation: SelectedPathObservation,
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    let path = match observation.path {
        PathKind::Unknown => v1::TerminalConnectionPath::Unknown,
        PathKind::Direct => v1::TerminalConnectionPath::Direct,
        PathKind::Relay => v1::TerminalConnectionPath::Relay,
    };
    let message = v1::TerminalConnectionStatusEvent {
        attachment_id: Some(attachment_id.into()),
        path: path as i32,
        rtt_ms: observation.rtt_ms,
    };
    let bytes = encode_message(WireKind::TerminalConnectionStatusEvent, 0, 0, &message)
        .map_err(protocol_error)?;
    write_local(writer, &bytes, deadline).await
}

async fn write_unsupported_history_gap<Writer>(
    frame: DecodedFrame,
    state: &BridgeState,
    writer: &mut Writer,
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    let request: v1::TerminalHistoryRequest = decode(&frame)?;
    require_local_attachment(request.attachment_id.clone(), state.local_view_id)?;
    validate_history_request(&request)?;
    let page = v1::TerminalHistoryPage {
        attachment_id: Some(state.local_view_id.into()),
        outcome: v1::TerminalHistoryOutcome::Gap as i32,
        cursor: None,
        rows: Vec::new(),
        current_epoch: 0,
        current_revision: 0,
    };
    let bytes = encode_message(
        WireKind::TerminalHistoryPage,
        frame.request_id,
        frame.deadline_ms,
        &page,
    )
    .map_err(protocol_error)?;
    write_local(writer, &bytes, deadline).await
}

async fn write_local<Writer>(
    writer: &mut Writer,
    bytes: &[u8],
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    timeout_at(
        deadline,
        writer.write_all(bytes),
        "local terminal write exceeded its absolute deadline",
    )
    .await?
    .map_err(|_| DaemonError::new(DomainErrorKind::Cancelled, "local terminal view closed"))?;
    timeout_at(
        deadline,
        writer.flush(),
        "local terminal flush exceeded its absolute deadline",
    )
    .await?
    .map_err(|_| DaemonError::new(DomainErrorKind::Cancelled, "local terminal view closed"))
}

async fn write_remote<Writer>(
    writer: &mut Writer,
    bytes: &[u8],
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    timeout_at(
        deadline,
        writer.write_all(bytes),
        "remote terminal write exceeded its absolute deadline",
    )
    .await?
    .map_err(|_| transport_unavailable("remote terminal stream write failed"))?;
    timeout_at(
        deadline,
        writer.flush(),
        "remote terminal flush exceeded its absolute deadline",
    )
    .await?
    .map_err(|_| transport_unavailable("remote terminal stream flush failed"))
}

async fn write_service_error<Writer>(
    writer: &mut Writer,
    request_id: u64,
    error: &DaemonError,
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    let bytes = ServiceReply::error(request_id, error).bytes;
    write_local(writer, &bytes, deadline).await
}

async fn write_error_best_effort<Writer>(
    writer: &mut Writer,
    request_id: u64,
    error: &DaemonError,
    deadline: Instant,
) where
    Writer: AsyncWrite + Unpin,
{
    let _ = write_service_error(writer, request_id, error, deadline).await;
}

async fn timeout_at<F>(
    deadline: Instant,
    future: F,
    detail: &'static str,
) -> Result<F::Output, DaemonError>
where
    F: Future,
{
    if Instant::now() >= deadline {
        return Err(DaemonError::new(DomainErrorKind::DeadlineExceeded, detail));
    }
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future)
        .await
        .map_err(|_| DaemonError::new(DomainErrorKind::DeadlineExceeded, detail))
}

fn random_resume_view_id() -> ResumeViewId {
    let secret = SecretKey::generate().to_bytes();
    let mut bytes = [0_u8; ResumeViewId::LENGTH];
    bytes.copy_from_slice(&secret[..ResumeViewId::LENGTH]);
    ResumeViewId::from_array(bytes)
}

fn is_temporary_transport(kind: DomainErrorKind) -> bool {
    matches!(
        kind,
        DomainErrorKind::TransportUnavailable
            | DomainErrorKind::AddressUnavailable
            | DomainErrorKind::DeadlineExceeded
            | DomainErrorKind::DaemonStopped
    )
}

fn transport_unavailable(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::TransportUnavailable, detail)
}

fn malformed(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::time::Duration;

    use tokio::io::{AsyncWriteExt, DuplexStream, duplex};
    use tokio::net::UnixStream;
    use tokio::sync::{mpsc, oneshot};
    use zterm_core::{
        AuthGeneration, AuthorizationStatus, DaemonIncarnation, DeviceDisplayName, OperationId,
        OperationLease, ResourceLimits, SessionName, SessionSelector,
    };
    use zterm_platform::pty::{ExplicitPtyCommand, PtyHost, PtySize};

    use super::*;
    use crate::authorization::AuthorizationRegistry;
    use crate::device_directory::ResolvedSessionTarget;
    use crate::local_ipc::{LocalAttachmentClient, LocalAttachmentEvent};
    use crate::session::{AttachmentLifecycle, SessionService};
    use crate::session_wire::{SessionRequestContext, SessionWireServer, read_first};
    use crate::store::DeviceAuthorization;

    #[test]
    fn epoch_end_debug_retains_only_the_typed_error_category() {
        const ERROR_SENTINEL: &str = "REMOTE_ERROR_TEXT_SENTINEL_1f8c";
        let end = EpochEnd::Reconnect(DaemonError::new(
            DomainErrorKind::TransportUnavailable,
            ERROR_SENTINEL,
        ));
        let rendered = format!("{end:?}");

        assert!(!rendered.contains(ERROR_SENTINEL));
        assert!(rendered.contains("TransportUnavailable"));
        assert_eq!(
            end,
            EpochEnd::Reconnect(DaemonError::new(
                DomainErrorKind::TransportUnavailable,
                ERROR_SENTINEL,
            ))
        );
    }

    #[derive(Clone)]
    struct FakeTransport {
        state: Arc<Mutex<FakeTransportState>>,
    }

    struct FakeTransportState {
        demand_calls: usize,
        active_demands: usize,
        demand_drops: usize,
        open_calls: usize,
        owned_targets: BTreeSet<DeviceId>,
        opens: VecDeque<FakeOpen>,
    }

    enum FakeOpen {
        Stream(BoxAttachmentStream),
        Observed(BoxAttachmentStream, Arc<dyn RemoteAttachmentEpochObserver>),
        Error(DomainErrorKind),
        Pending(oneshot::Sender<()>),
        After(oneshot::Receiver<()>, BoxAttachmentStream),
    }

    #[derive(Default)]
    struct FakeEpochObserver {
        history_paging: bool,
        path_observations: Mutex<VecDeque<SelectedPathObservation>>,
    }

    impl FakeEpochObserver {
        fn with_history_and_paths(
            history_paging: bool,
            observations: impl IntoIterator<Item = SelectedPathObservation>,
        ) -> Self {
            Self {
                history_paging,
                path_observations: Mutex::new(observations.into_iter().collect()),
            }
        }

        fn remaining_path_observations(&self) -> usize {
            self.path_observations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        }
    }

    impl RemoteAttachmentEpochObserver for FakeEpochObserver {
        fn selected_path_observation(&self) -> SelectedPathObservation {
            self.path_observations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front()
                .unwrap_or_default()
        }

        fn supports(&self, capability: u64) -> bool {
            capability == Capabilities::HISTORY_PAGING && self.history_paging
        }
    }

    #[test]
    fn pending_control_window_is_exact_and_recovers_after_correlation() {
        let mut pending = BTreeMap::new();
        for request_id in 1..=MAX_PENDING_CONTROL_REQUESTS {
            retain_pending(
                &mut pending,
                u64::try_from(request_id).expect("pending request ID fits u64"),
                WireKind::SessionOperationLeaseResponse,
            )
            .expect("every production pending-control slot is admitted");
        }
        assert_eq!(pending.len(), MAX_PENDING_CONTROL_REQUESTS);

        let overflow_id =
            u64::try_from(MAX_PENDING_CONTROL_REQUESTS + 1).expect("overflow request ID fits u64");
        let error = retain_pending(
            &mut pending,
            overflow_id,
            WireKind::SessionOperationLeaseResponse,
        )
        .expect_err("the next pending control exceeds the exact response window");
        assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);

        assert_eq!(
            pending.remove(&1),
            Some(WireKind::SessionOperationLeaseResponse),
            "a correlated response reaps exactly its pending cell",
        );
        retain_pending(
            &mut pending,
            overflow_id,
            WireKind::SessionOperationLeaseResponse,
        )
        .expect("reaping one correlation recovers one pending-control slot");
        assert_eq!(pending.len(), MAX_PENDING_CONTROL_REQUESTS);
    }

    #[tokio::test]
    async fn peer_without_history_capability_returns_gap_and_keeps_epoch_active() {
        let target = device(0x31);
        let session_id = session(0x32);
        let local_id = attachment(0x33);
        let remote_id = attachment(0x34);
        let revision = Revision::new(5);
        let (remote_bridge, remote_host) = duplex(64 * 1024);
        let remote_bridge: BoxAttachmentStream = Box::new(remote_bridge);
        let (remote_read, remote_write) = tokio::io::split(remote_bridge);
        let epoch = RemoteEpoch {
            reader: FramedReader::fresh(remote_read),
            writer: remote_write,
            observer: Arc::new(FakeEpochObserver::default()),
            attachment_id: remote_id,
            initial_revision: revision,
        };
        let (local_cli, local_bridge) = duplex(64 * 1024);
        let (local_read, mut local_writer) = tokio::io::split(local_bridge);
        let mut local_reader = FramedReader::fresh(local_read);
        let mut state = bridge_state(target, local_id, session_id, revision);
        let task = tokio::spawn(async move {
            let mut desired = DesiredViewPhase::Synchronizing;
            run_epoch(
                epoch,
                &mut local_reader,
                &mut local_writer,
                &mut state,
                &mut desired,
                Duration::from_secs(1),
            )
            .await
        });

        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);
        let (remote_read, _remote_write) = tokio::io::split(remote_host);
        let mut remote_read = FramedReader::fresh(remote_read);

        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            2,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: revision.get(),
            },
        )
        .await;
        let acknowledgement = next_frame(&mut remote_read).await;
        assert_eq!(acknowledgement.kind, WireKind::TerminalSnapshotApplied);
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;

        write_message(
            &mut local_write,
            WireKind::TerminalHistoryRequest,
            3,
            &v1::TerminalHistoryRequest {
                attachment_id: Some(local_id.into()),
                direction: v1::TerminalHistoryDirection::Newest as i32,
                cursor: None,
                maximum_rows: 10,
            },
        )
        .await;
        let gap = next_non_status_frame(&mut local_read, local_id).await;
        assert_eq!(gap.kind, WireKind::TerminalHistoryPage);
        assert_eq!(gap.request_id, 3);
        let gap: v1::TerminalHistoryPage = gap
            .decode_message(WireKind::TerminalHistoryPage)
            .expect("unsupported-history gap page");
        assert_eq!(
            v1::TerminalHistoryOutcome::try_from(gap.outcome).expect("known history outcome"),
            v1::TerminalHistoryOutcome::Gap
        );
        assert!(gap.cursor.is_none());
        assert!(gap.rows.is_empty());
        assert_eq!((gap.current_epoch, gap.current_revision), (0, 0));

        write_message(
            &mut local_write,
            WireKind::TerminalInput,
            4,
            &v1::TerminalInput {
                operation_id: None,
                attachment_id: Some(local_id.into()),
                bytes: b"still-active".to_vec(),
            },
        )
        .await;
        let input = next_frame(&mut remote_read).await;
        assert_eq!(
            input.kind,
            WireKind::TerminalInput,
            "no unsupported history frame may reach the old peer"
        );
        let input: v1::TerminalInput = input
            .decode_message(WireKind::TerminalInput)
            .expect("post-gap terminal input");
        assert_eq!(input.bytes, b"still-active");
        assert_eq!(
            required_attachment_id(input.attachment_id).expect("rewritten remote attachment ID"),
            remote_id
        );

        write_message(
            &mut local_write,
            WireKind::TerminalDetach,
            5,
            &v1::TerminalDetach {
                attachment_id: Some(local_id.into()),
            },
        )
        .await;
        assert_eq!(
            next_frame(&mut remote_read).await.kind,
            WireKind::TerminalDetach
        );
        assert_eq!(
            task.await
                .expect("old-peer epoch task")
                .expect("old-peer gap keeps the epoch valid"),
            EpochEnd::Detached
        );
    }

    #[tokio::test(start_paused = true)]
    async fn active_epoch_emits_only_changed_direct_relay_and_unknown_samples_once_per_tick() {
        let target = device(0x35);
        let session_id = session(0x36);
        let local_id = attachment(0x37);
        let remote_id = attachment(0x38);
        let revision = Revision::new(8);
        let direct = SelectedPathObservation {
            path: PathKind::Direct,
            rtt_ms: Some(7),
        };
        let relay = SelectedPathObservation {
            path: PathKind::Relay,
            rtt_ms: Some(19),
        };
        let unknown = SelectedPathObservation::default();
        let observer = Arc::new(FakeEpochObserver::with_history_and_paths(
            false,
            [direct, direct, relay, unknown],
        ));

        let (remote_bridge, remote_host) = duplex(64 * 1024);
        let remote_bridge: BoxAttachmentStream = Box::new(remote_bridge);
        let (remote_read, remote_write) = tokio::io::split(remote_bridge);
        let epoch = RemoteEpoch {
            reader: FramedReader::fresh(remote_read),
            writer: remote_write,
            observer: observer.clone(),
            attachment_id: remote_id,
            initial_revision: revision,
        };
        let (local_cli, local_bridge) = duplex(64 * 1024);
        let (local_read, mut local_writer) = tokio::io::split(local_bridge);
        let mut local_reader = FramedReader::fresh(local_read);
        let mut state = bridge_state(target, local_id, session_id, revision);
        let task = tokio::spawn(async move {
            let mut desired = DesiredViewPhase::Synchronizing;
            run_epoch(
                epoch,
                &mut local_reader,
                &mut local_writer,
                &mut state,
                &mut desired,
                Duration::from_secs(1),
            )
            .await
        });

        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);
        let (remote_read, _remote_write) = tokio::io::split(remote_host);
        let mut remote_read = FramedReader::fresh(remote_read);
        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            2,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: revision.get(),
            },
        )
        .await;
        assert_eq!(
            next_paused_frame(&mut remote_read).await.kind,
            WireKind::TerminalSnapshotApplied
        );
        expect_paused_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;

        let first = expect_paused_connection_status(&mut local_read, local_id).await;
        assert_eq!(
            v1::TerminalConnectionPath::try_from(first.path).expect("known direct path"),
            v1::TerminalConnectionPath::Direct
        );
        assert_eq!(first.rtt_ms, Some(7));

        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            observer.remaining_path_observations(),
            2,
            "the unchanged sample is consumed without emitting a duplicate"
        );

        tokio::time::advance(Duration::from_secs(1)).await;
        let migrated = expect_paused_connection_status(&mut local_read, local_id).await;
        assert_eq!(
            v1::TerminalConnectionPath::try_from(migrated.path).expect("known relay path"),
            v1::TerminalConnectionPath::Relay
        );
        assert_eq!(migrated.rtt_ms, Some(19));

        tokio::time::advance(Duration::from_secs(1)).await;
        let disappeared = expect_paused_connection_status(&mut local_read, local_id).await;
        assert_eq!(
            v1::TerminalConnectionPath::try_from(disappeared.path).expect("known unknown path"),
            v1::TerminalConnectionPath::Unknown
        );
        assert_eq!(disappeared.rtt_ms, None);
        let redacted = format!(
            "{:?}",
            LocalAttachmentEvent::ConnectionStatus(disappeared.clone())
        );
        assert!(!redacted.contains(&target.to_string()));

        write_message(
            &mut local_write,
            WireKind::TerminalDetach,
            3,
            &v1::TerminalDetach {
                attachment_id: Some(local_id.into()),
            },
        )
        .await;
        assert_eq!(
            next_paused_frame(&mut remote_read).await.kind,
            WireKind::TerminalDetach
        );
        assert_eq!(
            task.await
                .expect("selected-path epoch task")
                .expect("selected-path samples keep the epoch valid"),
            EpochEnd::Detached
        );
    }

    struct FakeDemand {
        state: Arc<Mutex<FakeTransportState>>,
    }

    struct PendingDemandTransport {
        started: Mutex<Option<oneshot::Sender<()>>>,
    }

    impl FakeTransport {
        fn scripted(opens: impl IntoIterator<Item = FakeOpen>) -> Self {
            Self {
                state: Arc::new(Mutex::new(FakeTransportState {
                    demand_calls: 0,
                    active_demands: 0,
                    demand_drops: 0,
                    open_calls: 0,
                    owned_targets: BTreeSet::new(),
                    opens: opens.into_iter().collect(),
                })),
            }
        }

        fn state(&self) -> MutexGuard<'_, FakeTransportState> {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }

    impl RemoteAttachmentTransport for FakeTransport {
        fn demand<'a>(
            &'a self,
            target: DeviceId,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<Box<dyn RemoteAttachmentDemand>, DaemonError>> {
            Box::pin(async move {
                let mut state = self.state();
                state.demand_calls += 1;
                state.active_demands += 1;
                state.owned_targets.insert(target);
                drop(state);
                Ok(Box::new(FakeDemand {
                    state: Arc::clone(&self.state),
                }) as Box<dyn RemoteAttachmentDemand>)
            })
        }
    }

    impl RemoteAttachmentTransport for PendingDemandTransport {
        fn demand<'a>(
            &'a self,
            _target: DeviceId,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<Box<dyn RemoteAttachmentDemand>, DaemonError>> {
            let started = self
                .started
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            Box::pin(async move {
                if let Some(started) = started {
                    let _ = started.send(());
                }
                std::future::pending::<Result<Box<dyn RemoteAttachmentDemand>, DaemonError>>().await
            })
        }
    }

    impl RemoteAttachmentDemand for FakeDemand {
        fn open<'a>(
            &'a mut self,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<OpenedAttachmentEpoch, DaemonError>> {
            let open = {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.open_calls += 1;
                state.opens.pop_front()
            };
            Box::pin(async move {
                match open {
                    Some(FakeOpen::Stream(stream)) => Ok(OpenedAttachmentEpoch::unobserved(stream)),
                    Some(FakeOpen::Observed(stream, observer)) => {
                        Ok(OpenedAttachmentEpoch { stream, observer })
                    }
                    Some(FakeOpen::Error(kind)) => {
                        Err(DaemonError::new(kind, "scripted attachment open failure"))
                    }
                    Some(FakeOpen::Pending(started)) => {
                        let _ = started.send(());
                        std::future::pending::<Result<OpenedAttachmentEpoch, DaemonError>>().await
                    }
                    Some(FakeOpen::After(released, stream)) => {
                        released.await.map_err(|_| {
                            DaemonError::new(
                                DomainErrorKind::Cancelled,
                                "test host detach barrier was dropped",
                            )
                        })?;
                        Ok(OpenedAttachmentEpoch::unobserved(stream))
                    }
                    None => Err(DaemonError::new(
                        DomainErrorKind::ResourceExhausted,
                        "fake attachment transport has no scripted stream",
                    )),
                }
            })
        }
    }

    impl Drop for FakeDemand {
        fn drop(&mut self) {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.active_demands = state.active_demands.saturating_sub(1);
            state.demand_drops += 1;
        }
    }

    #[tokio::test]
    async fn opened_epoch_observer_remains_bound_across_primary_replacement() {
        let direct = SelectedPathObservation {
            path: PathKind::Direct,
            rtt_ms: Some(7),
        };
        let relay = SelectedPathObservation {
            path: PathKind::Relay,
            rtt_ms: Some(19),
        };
        let candidate_a = Arc::new(FakeEpochObserver::with_history_and_paths(
            false,
            [direct, direct],
        ));
        let candidate_b = Arc::new(FakeEpochObserver::with_history_and_paths(true, [relay]));
        let (first_stream, _first_peer) = duplex(1024);
        let (second_stream, _second_peer) = duplex(1024);
        let transport = FakeTransport::scripted([
            FakeOpen::Observed(Box::new(first_stream), candidate_a),
            FakeOpen::Observed(Box::new(second_stream), candidate_b),
        ]);
        let mut demand = FakeDemand {
            state: Arc::clone(&transport.state),
        };

        let first = demand
            .open(Instant::now() + Duration::from_secs(1))
            .await
            .expect("candidate A opens the first epoch");
        assert!(!first.observer.supports(Capabilities::HISTORY_PAGING));
        assert_eq!(first.observer.selected_path_observation(), direct);

        let second = demand
            .open(Instant::now() + Duration::from_secs(1))
            .await
            .expect("candidate B replaces the primary for the next epoch");
        assert!(second.observer.supports(Capabilities::HISTORY_PAGING));
        assert_eq!(second.observer.selected_path_observation(), relay);
        assert!(
            !first.observer.supports(Capabilities::HISTORY_PAGING),
            "candidate B cannot change candidate A's open-epoch capability gate"
        );
        assert_eq!(
            first.observer.selected_path_observation(),
            direct,
            "candidate B cannot change candidate A's open-epoch path observer"
        );
    }

    struct FirstEpochEvidence {
        attach: v1::TerminalAttachRequest,
        acknowledgement: v1::TerminalSnapshotApplied,
        resize: v1::TerminalResize,
        input: v1::TerminalInput,
    }

    struct SecondEpochEvidence {
        attach: v1::TerminalAttachRequest,
        acknowledgement: v1::TerminalSnapshotApplied,
        resize: v1::TerminalResize,
        input: v1::TerminalInput,
        detach: v1::TerminalDetach,
    }

    struct ResumedEpochEvidence {
        attach: v1::TerminalAttachRequest,
        acknowledgement: v1::TerminalSnapshotApplied,
        resize: v1::TerminalResize,
        detach: v1::TerminalDetach,
    }

    #[tokio::test]
    async fn unix_view_reconnects_with_one_demand_fresh_remote_ids_and_safe_sync_gating() {
        let target = device(0x21);
        let session_id = session(0x31);
        let local_id = attachment(0x41);
        let first_remote_id = attachment(0x51);
        let second_remote_id = attachment(0x52);
        let (first_bridge, first_host) = duplex(64 * 1024);
        let (second_bridge, second_host) = duplex(64 * 1024);
        let transport = FakeTransport::scripted([
            FakeOpen::Stream(Box::new(first_bridge)),
            FakeOpen::Stream(Box::new(second_bridge)),
        ]);
        let client = RemoteAttachmentClient {
            transport: Arc::new(transport.clone()),
        };
        let first_host = tokio::spawn(run_first_host_epoch(
            first_host,
            session_id,
            first_remote_id,
        ));
        let second_host = tokio::spawn(run_second_host_epoch(second_host, second_remote_id));

        let (mut local_cli, mut local_daemon) = UnixStream::pair().expect("Unix attachment pair");
        write_message(
            &mut local_cli,
            WireKind::TerminalAttachRequest,
            1,
            &attach_main(target),
        )
        .await;
        let first = read_first(&mut local_daemon)
            .await
            .expect("daemon routes one initial attachment frame");
        let serve = tokio::spawn(async move {
            client
                .serve(
                    target,
                    local_id,
                    local_daemon,
                    first,
                    SessionWireLimits::default(),
                    Instant::now() + Duration::from_secs(2),
                )
                .await
        });
        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);

        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Synchronizing,
        )
        .await;
        let snapshot = next_frame(&mut local_read).await;
        assert_eq!(snapshot.kind, WireKind::TerminalSnapshot);
        let snapshot: v1::TerminalSnapshot = snapshot
            .decode_message(WireKind::TerminalSnapshot)
            .expect("local initial snapshot");
        assert_eq!(
            required_attachment_id(snapshot.attachment_id).expect("local ID"),
            local_id
        );
        assert_eq!(
            required_session_id(snapshot.session_id).expect("session ID"),
            session_id
        );
        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            2,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: 5,
            },
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;
        write_message(
            &mut local_write,
            WireKind::TerminalInput,
            3,
            &v1::TerminalInput {
                operation_id: None,
                attachment_id: Some(local_id.into()),
                bytes: b"first-active".to_vec(),
            },
        )
        .await;

        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Reconnecting,
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Synchronizing,
        )
        .await;
        let delta = next_frame(&mut local_read).await;
        assert_eq!(delta.kind, WireKind::TerminalDelta);
        let delta: v1::TerminalDelta = delta
            .decode_message(WireKind::TerminalDelta)
            .expect("local resume delta");
        assert_eq!(
            required_attachment_id(delta.attachment_id).expect("local ID"),
            local_id
        );
        assert_eq!((delta.from_revision, delta.to_revision), (5, 6));

        write_message(
            &mut local_write,
            WireKind::TerminalInput,
            4,
            &v1::TerminalInput {
                operation_id: None,
                attachment_id: Some(local_id.into()),
                bytes: b"must-drop".to_vec(),
            },
        )
        .await;
        for (request_id, rows, columns) in [(5, 40, 100), (6, 60, 120)] {
            write_message(
                &mut local_write,
                WireKind::TerminalResize,
                request_id,
                &v1::TerminalResize {
                    operation_id: None,
                    attachment_id: Some(local_id.into()),
                    rows,
                    columns,
                },
            )
            .await;
        }
        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            7,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: 6,
            },
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;
        write_message(
            &mut local_write,
            WireKind::TerminalInput,
            8,
            &v1::TerminalInput {
                operation_id: None,
                attachment_id: Some(local_id.into()),
                bytes: b"second-active".to_vec(),
            },
        )
        .await;
        write_message(
            &mut local_write,
            WireKind::TerminalDetach,
            9,
            &v1::TerminalDetach {
                attachment_id: Some(local_id.into()),
            },
        )
        .await;

        tokio::time::timeout(Duration::from_secs(2), serve)
            .await
            .expect("bridge stops after explicit detach")
            .expect("bridge task")
            .expect("explicit detach succeeds");
        let first = tokio::time::timeout(Duration::from_secs(2), first_host)
            .await
            .expect("first host epoch completes")
            .expect("first host task");
        let second = tokio::time::timeout(Duration::from_secs(2), second_host)
            .await
            .expect("second host epoch completes")
            .expect("second host task");

        assert!(first.attach.create_main);
        assert!(first.attach.session_id.is_none());
        assert_eq!(
            first
                .attach
                .viewport
                .expect("initial viewport is part of first atomic attach"),
            v1::TerminalViewport {
                rows: 24,
                columns: 80,
            }
        );
        assert!(!second.attach.create_main);
        assert_eq!(
            required_session_id(second.attach.session_id).expect("frozen SessionId"),
            session_id
        );
        assert!(second.attach.viewport.is_none());
        assert_eq!(second.attach.known_revision, Some(5));
        assert_eq!(first.attach.resume_view_id, second.attach.resume_view_id);
        assert_ne!(first_remote_id, second_remote_id);
        assert_eq!(
            required_attachment_id(first.acknowledgement.attachment_id)
                .expect("first remote acknowledgement ID"),
            first_remote_id
        );
        assert_eq!(
            required_attachment_id(second.acknowledgement.attachment_id)
                .expect("second remote acknowledgement ID"),
            second_remote_id
        );
        assert_eq!((first.resize.rows, first.resize.columns), (24, 80));
        assert_eq!((second.resize.rows, second.resize.columns), (60, 120));
        assert_eq!(first.input.bytes, b"first-active");
        assert_eq!(second.input.bytes, b"second-active");
        assert_eq!(
            required_attachment_id(second.detach.attachment_id).expect("remote detach ID"),
            second_remote_id
        );
        let state = transport.state();
        assert_eq!(state.demand_calls, 1);
        assert_eq!(state.open_calls, 2);
        assert_eq!(state.demand_drops, 1);
        assert_eq!(state.active_demands, 0);
        assert_eq!(state.owned_targets.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn active_view_retries_half_open_session_occupied_after_a_bounded_delay() {
        let target = device(0x22);
        let session_id = session(0x32);
        let local_id = attachment(0x42);
        let first_remote_id = attachment(0x53);
        let third_remote_id = attachment(0x54);
        let (first_bridge, first_host_stream) = duplex(64 * 1024);
        let (rejected_bridge, rejected_host_stream) = duplex(64 * 1024);
        let (third_bridge, third_host_stream) = duplex(64 * 1024);
        let transport = FakeTransport::scripted([
            FakeOpen::Stream(Box::new(first_bridge)),
            FakeOpen::Stream(Box::new(rejected_bridge)),
            FakeOpen::Stream(Box::new(third_bridge)),
        ]);
        let client = RemoteAttachmentClient {
            transport: Arc::new(transport.clone()),
        };

        let (first_half_open_tx, first_half_open_rx) = oneshot::channel();
        let (release_first_tx, release_first_rx) = oneshot::channel();
        let first_host = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(first_host_stream);
            let mut reader = FramedReader::fresh(reader);
            let attach: v1::TerminalAttachRequest = next_paused_frame(&mut reader)
                .await
                .decode_message(WireKind::TerminalAttachRequest)
                .expect("first overlap attach request");
            write_message(
                &mut writer,
                WireKind::TerminalSnapshot,
                1,
                &valid_snapshot(session_id, first_remote_id, 5),
            )
            .await;
            let acknowledgement: v1::TerminalSnapshotApplied = next_paused_frame(&mut reader)
                .await
                .decode_message(WireKind::TerminalSnapshotApplied)
                .expect("first overlap acknowledgement");
            let resize: v1::TerminalResize = next_paused_frame(&mut reader)
                .await
                .decode_message(WireKind::TerminalResize)
                .expect("first overlap resize");
            writer
                .shutdown()
                .await
                .expect("the controller side sees the first epoch disappear");
            let _ = first_half_open_tx.send(());
            let _ = release_first_rx.await;
            assert!(
                reader
                    .next()
                    .await
                    .expect("old owner observes a valid EOF")
                    .is_none(),
                "the old owner stays conceptually half-open until the test releases its reader"
            );
            (attach, acknowledgement, resize)
        });

        let (rejected_closed_tx, rejected_closed_rx) = oneshot::channel();
        let rejected_host = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(rejected_host_stream);
            let mut reader = FramedReader::fresh(reader);
            let attach: v1::TerminalAttachRequest = next_paused_frame(&mut reader)
                .await
                .decode_message(WireKind::TerminalAttachRequest)
                .expect("overlapping replacement attach request");
            writer
                .write_all(
                    &ServiceReply::error(
                        1,
                        &DaemonError::new(
                            DomainErrorKind::SessionOccupied,
                            "old host reader has not observed EOF",
                        ),
                    )
                    .bytes,
                )
                .await
                .expect("write overlapping SessionOccupied");
            writer
                .flush()
                .await
                .expect("flush overlapping SessionOccupied");
            assert!(
                reader
                    .next()
                    .await
                    .expect("rejected epoch closes with valid framing")
                    .is_none(),
                "the bridge drops the rejected stream epoch before retrying"
            );
            let _ = rejected_closed_tx.send(());
            attach
        });

        let (third_attach_tx, third_attach_rx) = oneshot::channel();
        let third_host = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(third_host_stream);
            let mut reader = FramedReader::fresh(reader);
            let attach: v1::TerminalAttachRequest = next_paused_frame(&mut reader)
                .await
                .decode_message(WireKind::TerminalAttachRequest)
                .expect("post-overlap attach request");
            let _ = third_attach_tx.send(tokio::time::Instant::now());
            write_message(
                &mut writer,
                WireKind::TerminalSnapshot,
                1,
                &valid_snapshot(session_id, third_remote_id, 6),
            )
            .await;
            let acknowledgement = next_paused_frame(&mut reader)
                .await
                .decode_message(WireKind::TerminalSnapshotApplied)
                .expect("post-overlap acknowledgement");
            let resize = next_paused_frame(&mut reader)
                .await
                .decode_message(WireKind::TerminalResize)
                .expect("post-overlap coalesced resize");
            let detach = next_paused_frame(&mut reader)
                .await
                .decode_message(WireKind::TerminalDetach)
                .expect("post-overlap detach");
            ResumedEpochEvidence {
                attach,
                acknowledgement,
                resize,
                detach,
            }
        });

        let (mut local_cli, mut local_daemon) = duplex(64 * 1024);
        write_message(
            &mut local_cli,
            WireKind::TerminalAttachRequest,
            1,
            &attach_main(target),
        )
        .await;
        let first = read_first(&mut local_daemon)
            .await
            .expect("daemon routes the overlap attachment frame");
        let serve = tokio::spawn(async move {
            client
                .serve(
                    target,
                    local_id,
                    local_daemon,
                    first,
                    SessionWireLimits::default(),
                    Instant::now() + Duration::from_secs(2),
                )
                .await
        });
        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);

        expect_paused_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        expect_paused_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Synchronizing,
        )
        .await;
        let first_snapshot = next_paused_frame(&mut local_read).await;
        assert_eq!(first_snapshot.kind, WireKind::TerminalSnapshot);
        let first_snapshot: v1::TerminalSnapshot = first_snapshot
            .decode_message(WireKind::TerminalSnapshot)
            .expect("first local overlap snapshot");
        assert_eq!(
            required_attachment_id(first_snapshot.attachment_id).expect("stable local ID"),
            local_id
        );
        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            2,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: 5,
            },
        )
        .await;
        expect_paused_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;
        first_half_open_rx
            .await
            .expect("old host reader remains conceptually half-open");
        expect_paused_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Reconnecting,
        )
        .await;
        let reconnect_status = expect_paused_connection_status(&mut local_read, local_id).await;
        assert_eq!(
            v1::TerminalConnectionPath::try_from(reconnect_status.path)
                .expect("known reconnect path"),
            v1::TerminalConnectionPath::Unknown
        );
        assert_eq!(reconnect_status.rtt_ms, None);
        rejected_closed_rx
            .await
            .expect("bridge drops the rejected replacement epoch");
        assert_eq!(transport.state().open_calls, 2);
        let retry_started = tokio::time::Instant::now();
        release_first_tx
            .send(())
            .expect("release the old owner during the retry delay");

        write_message(
            &mut local_write,
            WireKind::TerminalInput,
            3,
            &v1::TerminalInput {
                operation_id: None,
                attachment_id: Some(local_id.into()),
                bytes: b"drop-during-overlap".to_vec(),
            },
        )
        .await;
        for (request_id, rows, columns) in [(4, 40, 100), (5, 60, 120)] {
            write_message(
                &mut local_write,
                WireKind::TerminalResize,
                request_id,
                &v1::TerminalResize {
                    operation_id: None,
                    attachment_id: Some(local_id.into()),
                    rows,
                    columns,
                },
            )
            .await;
        }

        let final_tick = Duration::from_millis(1);
        tokio::time::advance(
            RESUME_OCCUPIED_RETRY_DELAY
                .checked_sub(final_tick)
                .expect("retry delay exceeds one test tick"),
        )
        .await;
        tokio::task::yield_now().await;
        assert_eq!(
            transport.state().open_calls,
            2,
            "the rejected epoch cannot trigger a hot-loop retry"
        );
        tokio::time::advance(final_tick).await;
        let third_attach_at = third_attach_rx
            .await
            .expect("bounded delay opens the replacement epoch");
        assert!(third_attach_at.duration_since(retry_started) >= RESUME_OCCUPIED_RETRY_DELAY);

        expect_paused_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Synchronizing,
        )
        .await;
        let third_snapshot = next_paused_frame(&mut local_read).await;
        assert_eq!(third_snapshot.kind, WireKind::TerminalSnapshot);
        let third_snapshot: v1::TerminalSnapshot = third_snapshot
            .decode_message(WireKind::TerminalSnapshot)
            .expect("post-overlap local snapshot");
        assert_eq!(
            required_attachment_id(third_snapshot.attachment_id).expect("stable local ID"),
            local_id
        );
        assert_eq!(
            required_session_id(third_snapshot.session_id).expect("frozen SessionId"),
            session_id
        );
        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            6,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: 6,
            },
        )
        .await;
        expect_paused_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;
        write_message(
            &mut local_write,
            WireKind::TerminalDetach,
            7,
            &v1::TerminalDetach {
                attachment_id: Some(local_id.into()),
            },
        )
        .await;

        serve
            .await
            .expect("overlap bridge task")
            .expect("post-overlap detach succeeds");
        let (first_attach, first_acknowledgement, first_resize) =
            first_host.await.expect("first overlap host task");
        let rejected_attach = rejected_host.await.expect("rejected overlap host task");
        let third = third_host.await.expect("post-overlap host task");

        assert!(first_attach.create_main);
        assert!(first_attach.session_id.is_none());
        for attach in [&rejected_attach, &third.attach] {
            assert!(!attach.create_main);
            assert!(attach.session_name.is_empty());
            assert_eq!(
                required_session_id(attach.session_id.clone()).expect("frozen SessionId"),
                session_id
            );
            assert_eq!(attach.known_revision, Some(5));
        }
        assert_eq!(first_attach.resume_view_id, rejected_attach.resume_view_id);
        assert_eq!(rejected_attach.resume_view_id, third.attach.resume_view_id);
        assert_ne!(first_remote_id, third_remote_id);
        assert_eq!(
            required_attachment_id(first_acknowledgement.attachment_id)
                .expect("first remote acknowledgement ID"),
            first_remote_id
        );
        assert_eq!(
            required_attachment_id(third.acknowledgement.attachment_id)
                .expect("fresh remote acknowledgement ID"),
            third_remote_id
        );
        assert_eq!((first_resize.rows, first_resize.columns), (24, 80));
        assert_eq!((third.resize.rows, third.resize.columns), (60, 120));
        assert_eq!(
            required_attachment_id(third.detach.attachment_id).expect("fresh remote detach ID"),
            third_remote_id
        );
        let state = transport.state();
        assert_eq!(state.demand_calls, 1);
        assert_eq!(state.open_calls, 3);
        assert_eq!(state.demand_drops, 1);
        assert_eq!(state.active_demands, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn first_ever_session_occupied_is_terminal() {
        let target = device(0x23);
        let local_id = attachment(0x43);
        let (bridge_stream, host_stream) = duplex(16 * 1024);
        let transport = FakeTransport::scripted([FakeOpen::Stream(Box::new(bridge_stream))]);
        let client = RemoteAttachmentClient {
            transport: Arc::new(transport.clone()),
        };
        let host = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(host_stream);
            let mut reader = FramedReader::fresh(reader);
            let attach: v1::TerminalAttachRequest = next_paused_frame(&mut reader)
                .await
                .decode_message(WireKind::TerminalAttachRequest)
                .expect("first-ever occupied attach request");
            writer
                .write_all(
                    &ServiceReply::error(
                        1,
                        &DaemonError::new(
                            DomainErrorKind::SessionOccupied,
                            "first attach is genuinely occupied",
                        ),
                    )
                    .bytes,
                )
                .await
                .expect("write first-ever SessionOccupied");
            writer
                .flush()
                .await
                .expect("flush first-ever SessionOccupied");
            attach
        });

        let (local_cli, task) = start_pending_view(client, target, local_id).await;
        let (local_read, _local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);
        expect_paused_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        let response = next_paused_frame(&mut local_read).await;
        assert_eq!(response.kind, WireKind::ServiceErrorResponse);
        assert_eq!(
            service_error_kind(&response),
            DomainErrorKind::SessionOccupied
        );
        let error = task
            .await
            .expect("first-ever occupied bridge task")
            .expect_err("first-ever SessionOccupied remains terminal");
        assert_eq!(error.kind(), DomainErrorKind::SessionOccupied);
        assert!(
            local_read
                .next()
                .await
                .expect("terminal local stream has valid framing")
                .is_none()
        );
        let attach = host.await.expect("first-ever occupied host task");
        assert!(attach.create_main);
        assert!(attach.session_id.is_none());
        let state = transport.state();
        assert_eq!(state.demand_calls, 1);
        assert_eq!(state.open_calls, 1);
        assert_eq!(state.demand_drops, 1);
        assert_eq!(state.active_demands, 0);
    }

    #[tokio::test]
    async fn reconnect_open_is_cancellable_and_two_views_share_one_task_private_target_owner() {
        let target = device(0x61);
        let first_id = attachment(0x62);
        let second_id = attachment(0x63);
        let (first_started_tx, first_started_rx) = oneshot::channel();
        let (second_started_tx, second_started_rx) = oneshot::channel();
        let transport = FakeTransport::scripted([
            FakeOpen::Error(DomainErrorKind::TransportUnavailable),
            FakeOpen::Pending(first_started_tx),
            FakeOpen::Pending(second_started_tx),
        ]);
        let client = RemoteAttachmentClient {
            transport: Arc::new(transport.clone()),
        };

        let (first_cli, first_task) = start_pending_view(client.clone(), target, first_id).await;
        let (first_read, mut first_write) = tokio::io::split(first_cli);
        let mut first_read = FramedReader::fresh(first_read);
        expect_transport_state(
            &mut first_read,
            first_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        expect_transport_state(
            &mut first_read,
            first_id,
            v1::TerminalTransportState::Reconnecting,
        )
        .await;
        first_started_rx
            .await
            .expect("first reconnect open started");

        let (second_cli, second_task) = start_pending_view(client, target, second_id).await;
        let (second_read, mut second_write) = tokio::io::split(second_cli);
        let mut second_read = FramedReader::fresh(second_read);
        expect_transport_state(
            &mut second_read,
            second_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        second_started_rx.await.expect("second view open started");
        {
            let state = transport.state();
            assert_eq!(state.demand_calls, 2);
            assert_eq!(state.active_demands, 2);
            assert_eq!(state.open_calls, 3);
            assert_eq!(state.owned_targets.len(), 1);
        }

        for (writer, attachment_id) in
            [(&mut first_write, first_id), (&mut second_write, second_id)]
        {
            write_message(
                writer,
                WireKind::TerminalDetach,
                9,
                &v1::TerminalDetach {
                    attachment_id: Some(attachment_id.into()),
                },
            )
            .await;
        }
        for task in [first_task, second_task] {
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .expect("pending open cancels with its local view")
                .expect("pending bridge task")
                .expect("offline detach succeeds");
        }
        let state = transport.state();
        assert_eq!(state.active_demands, 0);
        assert_eq!(state.demand_drops, 2);
    }

    #[tokio::test]
    async fn demand_acquisition_is_absolutely_bounded_and_locally_cancellable() {
        let target = device(0x64);
        let local_id = attachment(0x65);
        let (started_tx, started_rx) = oneshot::channel();
        let client = RemoteAttachmentClient {
            transport: Arc::new(PendingDemandTransport {
                started: Mutex::new(Some(started_tx)),
            }),
        };
        let (local_cli, task) = start_pending_view(client, target, local_id).await;
        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        started_rx
            .await
            .expect("pending demand acquisition started");
        write_message(
            &mut local_write,
            WireKind::TerminalDetach,
            2,
            &v1::TerminalDetach {
                attachment_id: Some(local_id.into()),
            },
        )
        .await;
        tokio::time::timeout(Duration::from_millis(250), task)
            .await
            .expect("local detach cancels a pending demand acquisition")
            .expect("pending-demand bridge task")
            .expect("offline detach succeeds");

        let (started_tx, started_rx) = oneshot::channel();
        let client = RemoteAttachmentClient {
            transport: Arc::new(PendingDemandTransport {
                started: Mutex::new(Some(started_tx)),
            }),
        };
        let (mut local_cli, mut local_daemon) =
            UnixStream::pair().expect("Unix demand-deadline pair");
        write_message(
            &mut local_cli,
            WireKind::TerminalAttachRequest,
            1,
            &attach_main(target),
        )
        .await;
        let first = read_first(&mut local_daemon)
            .await
            .expect("demand deadline first frame");
        let task = tokio::spawn(async move {
            client
                .serve(
                    target,
                    local_id,
                    local_daemon,
                    first,
                    SessionWireLimits::default(),
                    Instant::now() + Duration::from_millis(25),
                )
                .await
        });
        started_rx.await.expect("deadline-bound demand started");
        let error = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("pending demand releases at its absolute deadline")
            .expect("demand-deadline bridge task")
            .expect_err("pending demand returns a typed deadline failure");
        assert_eq!(error.kind(), DomainErrorKind::DeadlineExceeded);
    }

    #[tokio::test]
    async fn stalled_attach_write_and_initial_read_are_bounded_and_locally_cancellable() {
        let limits = SessionWireLimits::new(
            Duration::from_millis(50),
            Duration::from_millis(50),
            Duration::from_millis(50),
        );

        let target = device(0x54);
        let local_id = attachment(0x55);
        let (blocked_bridge, blocked_host) = duplex(1);
        let (reconnect_started_tx, reconnect_started_rx) = oneshot::channel();
        let transport = FakeTransport::scripted([
            FakeOpen::Stream(Box::new(blocked_bridge)),
            FakeOpen::Pending(reconnect_started_tx),
        ]);
        let client = RemoteAttachmentClient {
            transport: Arc::new(transport),
        };
        let (local_cli, task) = start_view_with_limits(client, target, local_id, limits).await;
        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Reconnecting,
        )
        .await;
        reconnect_started_rx
            .await
            .expect("attach-write timeout starts the next open");
        write_message(
            &mut local_write,
            WireKind::TerminalDetach,
            2,
            &v1::TerminalDetach {
                attachment_id: Some(local_id.into()),
            },
        )
        .await;
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("blocked attach write releases")
            .expect("attach-write bridge task")
            .expect("local detach cancels reconnect");
        drop(blocked_host);

        let target = device(0x56);
        let local_id = attachment(0x57);
        let (bridge_stream, host_stream) = duplex(16 * 1024);
        let (attach_seen_tx, attach_seen_rx) = oneshot::channel();
        let stalled_host = tokio::spawn(async move {
            let (reader, _writer) = tokio::io::split(host_stream);
            let mut reader = FramedReader::fresh(reader);
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalAttachRequest
            );
            let _ = attach_seen_tx.send(());
            std::future::pending::<()>().await;
        });
        let (reconnect_started_tx, reconnect_started_rx) = oneshot::channel();
        let transport = FakeTransport::scripted([
            FakeOpen::Stream(Box::new(bridge_stream)),
            FakeOpen::Pending(reconnect_started_tx),
        ]);
        let client = RemoteAttachmentClient {
            transport: Arc::new(transport),
        };
        let (local_cli, task) = start_view_with_limits(client, target, local_id, limits).await;
        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        attach_seen_rx.await.expect("host received bounded attach");
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Reconnecting,
        )
        .await;
        reconnect_started_rx
            .await
            .expect("initial-read timeout starts the next open");
        write_message(
            &mut local_write,
            WireKind::TerminalDetach,
            2,
            &v1::TerminalDetach {
                attachment_id: Some(local_id.into()),
            },
        )
        .await;
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("stalled initial read releases")
            .expect("initial-read bridge task")
            .expect("local detach cancels reconnect");
        stalled_host.abort();
        let _ = stalled_host.await;

        let limits = SessionWireLimits::new(
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_millis(100),
        );
        let target = device(0x5c);
        let local_id = attachment(0x5d);
        let session_id = session(0x5e);
        let first_remote_id = attachment(0x5f);
        let second_remote_id = attachment(0x60);
        let (first_bridge, first_host_stream) = duplex(16 * 1024);
        let (second_bridge, second_host_stream) = duplex(16 * 1024);
        let first_host = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(first_host_stream);
            let mut reader = FramedReader::fresh(reader);
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalAttachRequest
            );
            write_message(
                &mut writer,
                WireKind::TerminalSnapshot,
                1,
                &valid_snapshot(session_id, first_remote_id, 5),
            )
            .await;
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalSnapshotApplied
            );
            assert_eq!(next_frame(&mut reader).await.kind, WireKind::TerminalResize);
            writer
                .shutdown()
                .await
                .expect("lose fallback baseline epoch");
        });
        let (fallback_waiting_tx, fallback_waiting_rx) = oneshot::channel();
        let stalled_fallback = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(second_host_stream);
            let mut reader = FramedReader::fresh(reader);
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalAttachRequest
            );
            write_message(
                &mut writer,
                WireKind::TerminalDelta,
                1,
                &valid_delta(second_remote_id, 4, 6, b"force full"),
            )
            .await;
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalSyncRequest
            );
            let _ = fallback_waiting_tx.send(());
            std::future::pending::<()>().await;
        });
        let (third_open_tx, third_open_rx) = oneshot::channel();
        let transport = FakeTransport::scripted([
            FakeOpen::Stream(Box::new(first_bridge)),
            FakeOpen::Stream(Box::new(second_bridge)),
            FakeOpen::Pending(third_open_tx),
        ]);
        let client = RemoteAttachmentClient {
            transport: Arc::new(transport),
        };
        let (local_cli, task) = start_view_with_limits(client, target, local_id, limits).await;
        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Synchronizing,
        )
        .await;
        assert_eq!(
            next_frame(&mut local_read).await.kind,
            WireKind::TerminalSnapshot
        );
        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            2,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: 5,
            },
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Reconnecting,
        )
        .await;
        fallback_waiting_rx
            .await
            .expect("host withheld the required full-sync marker");
        third_open_rx
            .await
            .expect("full-fallback deadline released the stalled stream");
        write_message(
            &mut local_write,
            WireKind::TerminalDetach,
            3,
            &v1::TerminalDetach {
                attachment_id: Some(local_id.into()),
            },
        )
        .await;
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("stalled full fallback remains locally cancellable")
            .expect("full-fallback bridge task")
            .expect("local detach cancels full-fallback reconnect");
        first_host.await.expect("fallback baseline host task");
        stalled_fallback.abort();
        let _ = stalled_fallback.await;
    }

    #[tokio::test]
    async fn stalled_active_remote_write_releases_the_epoch_within_operation_timeout() {
        let limits = SessionWireLimits::new(
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_millis(100),
        );
        let target = device(0x58);
        let local_id = attachment(0x59);
        let session_id = session(0x5a);
        let remote_id = attachment(0x5b);
        let (bridge_stream, host_stream) = duplex(512);
        let (host_ready_tx, host_ready_rx) = oneshot::channel();
        let stalled_host = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(host_stream);
            let mut reader = FramedReader::fresh(reader);
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalAttachRequest
            );
            write_message(
                &mut writer,
                WireKind::TerminalSnapshot,
                1,
                &valid_snapshot(session_id, remote_id, 2),
            )
            .await;
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalSnapshotApplied
            );
            assert_eq!(next_frame(&mut reader).await.kind, WireKind::TerminalResize);
            let _ = host_ready_tx.send(());
            std::future::pending::<()>().await;
        });
        let (reconnect_started_tx, reconnect_started_rx) = oneshot::channel();
        let transport = FakeTransport::scripted([
            FakeOpen::Stream(Box::new(bridge_stream)),
            FakeOpen::Pending(reconnect_started_tx),
        ]);
        let client = RemoteAttachmentClient {
            transport: Arc::new(transport),
        };
        let (local_cli, task) = start_view_with_limits(client, target, local_id, limits).await;
        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Synchronizing,
        )
        .await;
        assert_eq!(
            next_frame(&mut local_read).await.kind,
            WireKind::TerminalSnapshot
        );
        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            2,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: 2,
            },
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;
        host_ready_rx
            .await
            .expect("host stopped reading after activation");
        write_message(
            &mut local_write,
            WireKind::TerminalInput,
            3,
            &v1::TerminalInput {
                operation_id: None,
                attachment_id: Some(local_id.into()),
                bytes: vec![b'x'; 32 * 1024],
            },
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Reconnecting,
        )
        .await;
        reconnect_started_rx
            .await
            .expect("write timeout releases the old stream and reopens");
        write_message(
            &mut local_write,
            WireKind::TerminalDetach,
            4,
            &v1::TerminalDetach {
                attachment_id: Some(local_id.into()),
            },
        )
        .await;
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("active write timeout remains locally cancellable")
            .expect("active-write bridge task")
            .expect("local detach cancels reconnect");
        stalled_host.abort();
        let _ = stalled_host.await;
    }

    #[tokio::test]
    async fn epoch_loss_resolves_sent_lease_and_takeover_without_replay() {
        let target = device(0x64);
        let session_id = session(0x65);
        let local_id = attachment(0x66);
        let remote_id = attachment(0x67);
        let (bridge_stream, host_stream) = duplex(64 * 1024);
        let (reconnect_started_tx, reconnect_started_rx) = oneshot::channel();
        let transport = FakeTransport::scripted([
            FakeOpen::Stream(Box::new(bridge_stream)),
            FakeOpen::Pending(reconnect_started_tx),
        ]);
        let client = RemoteAttachmentClient {
            transport: Arc::new(transport.clone()),
        };
        let host = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(host_stream);
            let mut reader = FramedReader::fresh(reader);
            let attach = next_frame(&mut reader).await;
            assert_eq!(attach.kind, WireKind::TerminalAttachRequest);
            write_message(
                &mut writer,
                WireKind::TerminalSnapshot,
                1,
                &valid_snapshot(session_id, remote_id, 3),
            )
            .await;
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalSnapshotApplied
            );
            assert_eq!(next_frame(&mut reader).await.kind, WireKind::TerminalResize);
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::SessionOperationLeaseRequest
            );
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::SessionTakeoverRequest
            );
            writer
                .shutdown()
                .await
                .expect("lose control-response epoch");
        });

        let (local_cli, bridge) = start_pending_view(client, target, local_id).await;
        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Synchronizing,
        )
        .await;
        let snapshot = next_frame(&mut local_read).await;
        assert_eq!(snapshot.kind, WireKind::TerminalSnapshot);
        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            2,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: 3,
            },
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;
        write_message(
            &mut local_write,
            WireKind::SessionOperationLeaseRequest,
            20,
            &v1::SessionOperationLeaseRequest {
                target: Some(device_target(target)),
            },
        )
        .await;
        write_message(
            &mut local_write,
            WireKind::SessionTakeoverRequest,
            21,
            &v1::SessionTakeoverRequest {
                operation_id: Some(fixture_operation_id().into()),
                target: Some(device_target(target)),
                session_id: Some(session_id.into()),
                attachment_id: Some(local_id.into()),
            },
        )
        .await;

        let lease_error = next_non_status_frame(&mut local_read, local_id).await;
        assert_eq!(lease_error.request_id, 20);
        assert_eq!(
            service_error_kind(&lease_error),
            DomainErrorKind::TransportUnavailable
        );
        let takeover_error = next_non_status_frame(&mut local_read, local_id).await;
        assert_eq!(takeover_error.request_id, 21);
        assert_eq!(
            service_error_kind(&takeover_error),
            DomainErrorKind::OperationOutcomeUnknown
        );
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Reconnecting,
        )
        .await;
        reconnect_started_rx
            .await
            .expect("bridge retained one demand for reconnect");
        write_message(
            &mut local_write,
            WireKind::TerminalDetach,
            22,
            &v1::TerminalDetach {
                attachment_id: Some(local_id.into()),
            },
        )
        .await;

        tokio::time::timeout(Duration::from_secs(2), bridge)
            .await
            .expect("pending reconnect cancels")
            .expect("bridge task")
            .expect("local detach succeeds");
        host.await.expect("control-loss host task");
        let state = transport.state();
        assert_eq!(state.open_calls, 2);
        assert_eq!(state.demand_drops, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn explicit_takeover_token_continues_the_host_replay_after_encoded_response_loss() {
        let host = device(0x68);
        let remote = device(0x69);
        let accepted_generation = AuthGeneration::new(9).expect("non-zero generation");
        let authorization = takeover_authorization(remote, accepted_generation);
        let temporary = tempfile::tempdir().expect("temporary takeover replay fixture");
        let spawn_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sessions = takeover_session_service(
            host,
            temporary.path().to_path_buf(),
            Arc::clone(&spawn_count),
        );

        let original_view = attachment(0x6a);
        let local_principal = sessions.local_principal(original_view);
        let local_lease = sessions
            .issue_operation_lease(local_principal)
            .expect("issue local fixture lease");
        let created = sessions
            .create(
                local_principal,
                OperationId {
                    lease: local_lease,
                    sequence: 1,
                },
                SessionName::new("takeover-replay").expect("fixture Session name"),
                None,
                Some(zterm_core::terminal::TerminalSize::new(24, 80)),
            )
            .expect("create takeover fixture Session");
        let original = sessions
            .prepare_attach(
                local_principal,
                Some(SessionSelector::Id(created.session_id)),
                false,
                false,
                None,
            )
            .expect("prepare original controller");
        original
            .attachment
            .snapshot_applied(original.snapshot.revision)
            .expect("activate original controller");
        let original_attachment = Arc::clone(&original.attachment);
        let original_lifecycle = original_attachment
            .lifecycle_watch()
            .expect("observe original controller lifecycle");
        assert!(matches!(
            *original_lifecycle.borrow(),
            AttachmentLifecycle::Active { .. }
        ));

        let server = SessionWireServer::new(sessions.clone());
        let context = SessionRequestContext::RemoteAuthenticated {
            own_device_id: host,
            remote_device_id: remote,
            accepted_generation,
            authorization,
            commit_first_poll_observer: None,
        };
        let (evidence_tx, mut evidence_rx) = mpsc::channel(2);
        let (first_detached_tx, first_detached_rx) = oneshot::channel();
        let (first_bridge, first_relay, first_server) = start_relayed_takeover_epoch(
            1,
            true,
            server.clone(),
            context.clone(),
            evidence_tx.clone(),
            Some(first_detached_tx),
        );
        let (second_bridge, second_relay, second_server) =
            start_relayed_takeover_epoch(2, false, server, context, evidence_tx, None);
        let transport = FakeTransport::scripted([
            FakeOpen::Stream(Box::new(first_bridge)),
            FakeOpen::After(first_detached_rx, Box::new(second_bridge)),
        ]);
        let remote_client = RemoteAttachmentClient {
            transport: Arc::new(transport.clone()),
        };

        let local_view = attachment(0x6b);
        let (mut local_client, local_daemon) = LocalAttachmentClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(host),
            created.session_id,
            local_view,
        );
        let first = decoded_first_frame(
            WireKind::TerminalAttachRequest,
            1,
            &v1::TerminalAttachRequest {
                target: Some(device_target(host)),
                session_id: Some(created.session_id.into()),
                takeover: true,
                session_name: String::new(),
                create_main: false,
                viewport: Some(v1::TerminalViewport {
                    rows: 24,
                    columns: 80,
                }),
                resume_view_id: None,
                known_revision: None,
            },
        );
        let bridge = tokio::spawn(async move {
            remote_client
                .serve(
                    host,
                    local_view,
                    local_daemon,
                    first,
                    SessionWireLimits::default(),
                    Instant::now() + Duration::from_secs(5),
                )
                .await
        });

        apply_next_bridge_initial(&mut local_client).await;
        let continuation = local_client
            .begin_takeover()
            .await
            .expect("send first takeover with one daemon-issued lease");
        let first_evidence = evidence_rx
            .recv()
            .await
            .expect("first host response was completely encoded before loss");
        assert_eq!(first_evidence.epoch, 1);
        assert_eq!(first_evidence.lease_requests, 1);
        let first_response = decode_one(&first_evidence.response_frame);
        assert_eq!(first_response.kind, WireKind::SessionMutateResponse);
        assert!(
            first_response.payload.as_slice() == first_evidence.response_payload.as_slice(),
            "captured first response retains its complete mutation payload"
        );
        let first_operation: OperationId = first_evidence
            .takeover
            .operation_id
            .clone()
            .expect("first takeover operation ID")
            .try_into()
            .expect("valid first takeover operation ID");
        assert_eq!(
            first_evidence
                .issued_lease
                .expect("first epoch relayed one daemon-issued lease"),
            first_operation.lease
        );
        let first_remote_attachment: AttachmentId = first_evidence
            .takeover
            .attachment_id
            .clone()
            .expect("first remote attachment ID")
            .try_into()
            .expect("valid first remote attachment ID");
        let lost_generation = match &*original_lifecycle.borrow() {
            AttachmentLifecycle::LeaseLost { generation } => *generation,
            lifecycle => panic!("original controller was not replaced exactly once: {lifecycle:?}"),
        };

        // Change a response field only after the first takeover has committed
        // and its complete response has crossed the relay barrier. A fresh
        // second execution would return this newer name; replay must retain
        // the exact earlier response while preserving the intervening state.
        sessions
            .rename(
                local_principal,
                OperationId {
                    lease: local_lease,
                    sequence: 2,
                },
                created.session_id,
                SessionName::new("renamed-after-takeover-loss").expect("intervening Session name"),
            )
            .expect("rename after the captured takeover response");
        assert_eq!(
            sessions.list().expect("list after intervening rename")[0]
                .name
                .as_str(),
            "renamed-after-takeover-loss"
        );

        loop {
            match local_client.read_event(Duration::from_secs(2)).await {
                Err(error) if error.kind() == DomainErrorKind::OperationOutcomeUnknown => break,
                Ok(LocalAttachmentEvent::TransportState(_)) => {}
                event => panic!("unexpected event before takeover outcome unknown: {event:?}"),
            }
        }

        apply_next_bridge_initial(&mut local_client).await;
        local_client
            .retry_takeover(continuation)
            .await
            .expect("explicit continuation sends the opaque prior operation");
        let second_evidence = evidence_rx
            .recv()
            .await
            .expect("continued host replay produced its response");
        assert_eq!(second_evidence.epoch, 2);
        let second_response = decode_one(&second_evidence.response_frame);
        assert_eq!(second_response.kind, WireKind::SessionMutateResponse);
        assert!(
            second_response.payload.as_slice() == second_evidence.response_payload.as_slice(),
            "captured continued response retains its complete mutation payload"
        );
        assert_eq!(
            second_evidence.lease_requests, 0,
            "continuation must not allocate a fresh lease"
        );
        let second_operation: OperationId = second_evidence
            .takeover
            .operation_id
            .clone()
            .expect("continued takeover operation ID")
            .try_into()
            .expect("valid continued takeover operation ID");
        assert_eq!(second_operation, first_operation);
        assert!(
            second_evidence.response_payload.as_slice()
                == first_evidence.response_payload.as_slice(),
            "the host reuses the exact retained mutation result"
        );
        let second_remote_attachment: AttachmentId = second_evidence
            .takeover
            .attachment_id
            .clone()
            .expect("continued remote attachment ID")
            .try_into()
            .expect("valid continued remote attachment ID");
        assert_ne!(
            second_remote_attachment, first_remote_attachment,
            "continuation applies the same operation to the newly synchronized attachment"
        );

        let takeover = loop {
            match local_client
                .read_event(Duration::from_secs(2))
                .await
                .expect("continued takeover event")
            {
                LocalAttachmentEvent::Takeover(summary) => break summary,
                LocalAttachmentEvent::TransportState(_)
                | LocalAttachmentEvent::Snapshot(_)
                | LocalAttachmentEvent::Delta(_)
                | LocalAttachmentEvent::SyncRequired(_) => {}
                event => panic!("unexpected continuation event: {event:?}"),
            }
        };
        assert_eq!(takeover.session_id, created.session_id);
        assert_eq!(takeover.name.as_str(), "takeover-replay");
        assert!(takeover.has_controller);
        assert!(matches!(
            *original_lifecycle.borrow(),
            AttachmentLifecycle::LeaseLost { generation } if generation == lost_generation
        ));
        let listed = sessions.list().expect("list continued takeover Session");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, created.session_id);
        assert_eq!(listed[0].name.as_str(), "renamed-after-takeover-loss");
        assert!(listed[0].has_controller);
        assert_eq!(spawn_count.load(std::sync::atomic::Ordering::Acquire), 1);

        local_client
            .detach()
            .await
            .expect("detach only the continued local view");
        tokio::time::timeout(Duration::from_secs(2), bridge)
            .await
            .expect("bridge detach completion")
            .expect("bridge task")
            .expect("bridge detach result");
        await_takeover_epoch_task(first_relay, "first relay").await;
        await_takeover_server_task(first_server, "first server").await;
        await_takeover_epoch_task(second_relay, "second relay").await;
        await_takeover_server_task(second_server, "second server").await;
        assert!(
            evidence_rx.try_recv().is_err(),
            "the completed replacement epoch contains only the explicit continuation"
        );

        let state = transport.state();
        assert_eq!(state.demand_calls, 1);
        assert_eq!(state.open_calls, 2);
        assert_eq!(state.demand_drops, 1);
        drop(state);
        sessions
            .close(
                local_principal,
                OperationId {
                    lease: local_lease,
                    sequence: 3,
                },
                created.session_id,
            )
            .expect("close takeover replay fixture Session");
    }

    #[tokio::test]
    async fn inconsistent_resume_delta_consumes_required_marker_then_full_snapshot() {
        let target = device(0x74);
        let session_id = session(0x75);
        let local_id = attachment(0x76);
        let first_remote_id = attachment(0x77);
        let second_remote_id = attachment(0x78);
        let (first_bridge, first_host_stream) = duplex(64 * 1024);
        let (second_bridge, second_host_stream) = duplex(64 * 1024);
        let transport = FakeTransport::scripted([
            FakeOpen::Stream(Box::new(first_bridge)),
            FakeOpen::Stream(Box::new(second_bridge)),
        ]);
        let client = RemoteAttachmentClient {
            transport: Arc::new(transport.clone()),
        };
        let first_host = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(first_host_stream);
            let mut reader = FramedReader::fresh(reader);
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalAttachRequest
            );
            write_message(
                &mut writer,
                WireKind::TerminalSnapshot,
                1,
                &valid_snapshot(session_id, first_remote_id, 5),
            )
            .await;
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalSnapshotApplied
            );
            assert_eq!(next_frame(&mut reader).await.kind, WireKind::TerminalResize);
            writer.shutdown().await.expect("lose first resume epoch");
        });
        let second_host = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(second_host_stream);
            let mut reader = FramedReader::fresh(reader);
            let attach = next_frame(&mut reader).await;
            let attach: v1::TerminalAttachRequest = attach
                .decode_message(WireKind::TerminalAttachRequest)
                .expect("second attach request");
            assert_eq!(attach.known_revision, Some(5));
            write_message(
                &mut writer,
                WireKind::TerminalDelta,
                1,
                &valid_delta(second_remote_id, 4, 6, b"inconsistent"),
            )
            .await;
            let sync = next_frame(&mut reader).await;
            assert_eq!(sync.kind, WireKind::TerminalSyncRequest);
            assert_eq!(sync.request_id, 1);
            let sync: v1::TerminalSyncRequest = sync
                .decode_message(WireKind::TerminalSyncRequest)
                .expect("full-sync request");
            assert_eq!(sync.known_revision, 5);
            assert_eq!(
                required_attachment_id(sync.attachment_id).expect("same epoch ID"),
                second_remote_id
            );
            write_message(
                &mut writer,
                WireKind::TerminalSyncRequired,
                1,
                &v1::TerminalSyncRequired {
                    attachment_id: Some(second_remote_id.into()),
                    latest_revision: 7,
                },
            )
            .await;
            write_message(
                &mut writer,
                WireKind::TerminalSnapshot,
                1,
                &valid_snapshot(session_id, second_remote_id, 7),
            )
            .await;
            let acknowledgement = next_frame(&mut reader).await;
            assert_eq!(acknowledgement.kind, WireKind::TerminalSnapshotApplied);
            let acknowledgement: v1::TerminalSnapshotApplied = acknowledgement
                .decode_message(WireKind::TerminalSnapshotApplied)
                .expect("fallback acknowledgement");
            assert_eq!(acknowledgement.revision, 7);
            assert_eq!(next_frame(&mut reader).await.kind, WireKind::TerminalResize);
            assert_eq!(next_frame(&mut reader).await.kind, WireKind::TerminalDetach);
        });

        let (local_cli, bridge) = start_pending_view(client, target, local_id).await;
        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Synchronizing,
        )
        .await;
        assert_eq!(
            next_frame(&mut local_read).await.kind,
            WireKind::TerminalSnapshot
        );
        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            2,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: 5,
            },
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Reconnecting,
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Synchronizing,
        )
        .await;
        let fallback = next_frame(&mut local_read).await;
        assert_eq!(
            fallback.kind,
            WireKind::TerminalSnapshot,
            "the bridge consumes the host-only SyncRequired marker"
        );
        let fallback: v1::TerminalSnapshot = fallback
            .decode_message(WireKind::TerminalSnapshot)
            .expect("local fallback snapshot");
        assert_eq!(fallback.revision, 7);
        assert_eq!(
            required_attachment_id(fallback.attachment_id).expect("stable local ID"),
            local_id
        );
        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            3,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: 7,
            },
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;
        write_message(
            &mut local_write,
            WireKind::TerminalDetach,
            4,
            &v1::TerminalDetach {
                attachment_id: Some(local_id.into()),
            },
        )
        .await;

        tokio::time::timeout(Duration::from_secs(2), bridge)
            .await
            .expect("fallback bridge stops")
            .expect("bridge task")
            .expect("fallback detach succeeds");
        first_host.await.expect("first fallback host task");
        second_host.await.expect("second fallback host task");
        assert_eq!(transport.state().open_calls, 2);
    }

    #[tokio::test]
    async fn fatal_epoch_protocol_error_is_flushed_locally_before_close() {
        let target = device(0x79);
        let session_id = session(0x7a);
        let local_id = attachment(0x7b);
        let remote_id = attachment(0x7c);
        let (bridge_stream, host_stream) = duplex(64 * 1024);
        let transport = FakeTransport::scripted([FakeOpen::Stream(Box::new(bridge_stream))]);
        let client = RemoteAttachmentClient {
            transport: Arc::new(transport),
        };
        let host = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(host_stream);
            let mut reader = FramedReader::fresh(reader);
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalAttachRequest
            );
            write_message(
                &mut writer,
                WireKind::TerminalSnapshot,
                1,
                &valid_snapshot(session_id, remote_id, 2),
            )
            .await;
            assert_eq!(
                next_frame(&mut reader).await.kind,
                WireKind::TerminalSnapshotApplied
            );
            assert_eq!(next_frame(&mut reader).await.kind, WireKind::TerminalResize);
            let pending_lease = next_frame(&mut reader).await;
            assert_eq!(pending_lease.kind, WireKind::SessionOperationLeaseRequest);
            assert_eq!(pending_lease.request_id, 78);
            write_message(
                &mut writer,
                WireKind::TerminalDelta,
                77,
                &valid_delta(attachment(0x7d), 2, 3, b"stale epoch"),
            )
            .await;
        });

        let (local_cli, bridge) = start_pending_view(client, target, local_id).await;
        let (local_read, mut local_write) = tokio::io::split(local_cli);
        let mut local_read = FramedReader::fresh(local_read);
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Preparing,
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Synchronizing,
        )
        .await;
        assert_eq!(
            next_frame(&mut local_read).await.kind,
            WireKind::TerminalSnapshot
        );
        write_message(
            &mut local_write,
            WireKind::TerminalSnapshotApplied,
            2,
            &v1::TerminalSnapshotApplied {
                attachment_id: Some(local_id.into()),
                revision: 2,
            },
        )
        .await;
        expect_transport_state(
            &mut local_read,
            local_id,
            v1::TerminalTransportState::Active,
        )
        .await;
        write_message(
            &mut local_write,
            WireKind::SessionOperationLeaseRequest,
            78,
            &v1::SessionOperationLeaseRequest {
                target: Some(device_target(target)),
            },
        )
        .await;
        let error = next_non_status_frame(&mut local_read, local_id).await;
        assert_eq!(error.kind, WireKind::ServiceErrorResponse);
        assert_eq!(error.request_id, 77);
        assert_eq!(service_error_kind(&error), DomainErrorKind::MalformedFrame);
        let pending = next_non_status_frame(&mut local_read, local_id).await;
        assert_eq!(pending.kind, WireKind::ServiceErrorResponse);
        assert_eq!(pending.request_id, 78);
        assert_eq!(
            service_error_kind(&pending),
            DomainErrorKind::TransportUnavailable
        );
        tokio::time::timeout(Duration::from_secs(2), bridge)
            .await
            .expect("fatal protocol bridge closes")
            .expect("bridge task")
            .expect("typed fatal outcome was already projected");
        host.await.expect("fatal protocol host task");
    }

    #[tokio::test]
    async fn correlated_service_errors_keep_the_epoch_unless_the_kind_is_fatal() {
        const UNTRUSTED_MESSAGE: &str = "REMOTE_ATTACHMENT_ERROR_SENTINEL_9b31";
        let target = device(0x68);
        let local_id = attachment(0x69);
        let remote_id = attachment(0x6a);
        let session_id = session(0x6b);
        let mut state = bridge_state(target, local_id, session_id, Revision::new(4));
        let mut phase = EpochPhase::Active;
        let mut desired = DesiredViewPhase::Active;
        let (local_read, mut local_writer) = duplex(16 * 1024);
        let mut local_read = FramedReader::fresh(local_read);
        let mut remote_sink = tokio::io::sink();

        state
            .pending_control
            .insert(31, WireKind::SessionMutateResponse);
        let ordinary = decode_one(
            &ServiceReply::error(
                31,
                &DaemonError::new(DomainErrorKind::SessionOccupied, UNTRUSTED_MESSAGE),
            )
            .bytes,
        );
        assert!(
            process_epoch_remote_frame(
                ordinary,
                remote_id,
                &mut state,
                &mut local_writer,
                &mut remote_sink,
                EpochControl {
                    phase: &mut phase,
                    desired_phase: &mut desired,
                    operation_timeout: Duration::from_secs(1),
                },
            )
            .await
            .expect("ordinary correlated error is valid")
            .is_none()
        );
        let ordinary = next_frame(&mut local_read).await;
        assert_eq!(ordinary.request_id, 31);
        assert_eq!(
            service_error_kind(&ordinary),
            DomainErrorKind::SessionOccupied
        );
        assert_eq!(
            service_error_message(&ordinary),
            "remote Session request failed"
        );
        assert!(!service_error_message(&ordinary).contains(UNTRUSTED_MESSAGE));
        assert!(state.pending_control.is_empty());

        state
            .pending_control
            .insert(32, WireKind::SessionOperationLeaseResponse);
        let fatal = decode_one(
            &ServiceReply::error(
                32,
                &DaemonError::new(DomainErrorKind::AuthorizationRevoked, "revoked fixture"),
            )
            .bytes,
        );
        let end = process_epoch_remote_frame(
            fatal,
            remote_id,
            &mut state,
            &mut local_writer,
            &mut remote_sink,
            EpochControl {
                phase: &mut phase,
                desired_phase: &mut desired,
                operation_timeout: Duration::from_secs(1),
            },
        )
        .await
        .expect("fatal correlated error is projected");
        assert_eq!(end, Some(EpochEnd::Terminal));
        let fatal = next_frame(&mut local_read).await;
        assert_eq!(fatal.request_id, 32);
        assert_eq!(
            service_error_kind(&fatal),
            DomainErrorKind::AuthorizationRevoked
        );
        assert!(state.pending_control.is_empty());
    }

    #[tokio::test]
    async fn synchronization_phase_rejects_control_requests_without_stranding_waiters() {
        let target = device(0x6f);
        let local_id = attachment(0x70);
        let remote_id = attachment(0x71);
        let session_id = session(0x72);
        let mut state = bridge_state(target, local_id, session_id, Revision::new(4));
        let mut phase = EpochPhase::Synchronizing {
            expected: Revision::new(5),
            acknowledged: false,
            needs_takeover: false,
        };
        let mut desired = DesiredViewPhase::Synchronizing;
        let (local_read, mut local_writer) = duplex(16 * 1024);
        let mut local_read = FramedReader::fresh(local_read);
        let mut remote_sink = tokio::io::sink();

        let lease = decoded_message(
            WireKind::SessionOperationLeaseRequest,
            41,
            &v1::SessionOperationLeaseRequest {
                target: Some(device_target(target)),
            },
        );
        assert!(
            process_epoch_local_frame(
                lease,
                &mut remote_sink,
                remote_id,
                &mut state,
                &mut local_writer,
                EpochControl {
                    phase: &mut phase,
                    desired_phase: &mut desired,
                    operation_timeout: Duration::from_secs(1),
                },
            )
            .await
            .expect("unsynchronized lease request is typed")
            .is_none()
        );
        let error = next_frame(&mut local_read).await;
        assert_eq!(error.request_id, 41);
        assert_eq!(service_error_kind(&error), DomainErrorKind::NotSynchronized);
        assert!(state.pending_control.is_empty());
    }

    #[tokio::test]
    async fn active_full_snapshot_announces_synchronizing_once_before_state_bytes() {
        let target = device(0x6c);
        let local_id = attachment(0x6d);
        let remote_id = attachment(0x6e);
        let session_id = session(0x6f);
        let mut state = bridge_state(target, local_id, session_id, Revision::new(4));
        let mut phase = EpochPhase::Active;
        let mut desired = DesiredViewPhase::Active;
        let (local_read, mut local_writer) = duplex(16 * 1024);
        let mut local_read = FramedReader::fresh(local_read);
        let mut remote_sink = tokio::io::sink();

        for revision in [5, 6] {
            let snapshot = decoded_message(
                WireKind::TerminalSnapshot,
                0,
                &valid_snapshot(session_id, remote_id, revision),
            );
            assert!(
                process_epoch_remote_frame(
                    snapshot,
                    remote_id,
                    &mut state,
                    &mut local_writer,
                    &mut remote_sink,
                    EpochControl {
                        phase: &mut phase,
                        desired_phase: &mut desired,
                        operation_timeout: Duration::from_secs(1),
                    },
                )
                .await
                .expect("mid-epoch full snapshot is valid")
                .is_none()
            );
            if revision == 5 {
                expect_transport_state(
                    &mut local_read,
                    local_id,
                    v1::TerminalTransportState::Synchronizing,
                )
                .await;
            }
            let frame = next_frame(&mut local_read).await;
            assert_eq!(frame.kind, WireKind::TerminalSnapshot);
            let snapshot: v1::TerminalSnapshot = frame
                .decode_message(WireKind::TerminalSnapshot)
                .expect("local full snapshot");
            assert_eq!(snapshot.revision, revision);
        }
    }

    #[tokio::test]
    async fn revision_gaps_request_full_sync_and_terminal_failures_never_retry() {
        let target = device(0x71);
        let local_id = attachment(0x72);
        let remote_id = attachment(0x73);
        let session_id = session(0x74);
        let mut state = bridge_state(target, local_id, session_id, Revision::new(5));
        let mut phase = EpochPhase::Active;
        let mut desired = DesiredViewPhase::Active;
        let (remote_peer, mut remote_writer) = duplex(4 * 1024);
        let mut local_sink = tokio::io::sink();
        let gap = decoded_message(
            WireKind::TerminalDelta,
            0,
            &valid_delta(remote_id, 4, 6, b"gap"),
        );
        assert!(
            process_epoch_remote_frame(
                gap,
                remote_id,
                &mut state,
                &mut local_sink,
                &mut remote_writer,
                EpochControl {
                    phase: &mut phase,
                    desired_phase: &mut desired,
                    operation_timeout: Duration::from_secs(1),
                },
            )
            .await
            .expect("gap is recoverable")
            .is_none()
        );
        assert!(state.force_full);
        assert!(matches!(phase, EpochPhase::Synchronizing { .. }));
        let mut remote_peer = FramedReader::fresh(remote_peer);
        let sync = next_frame(&mut remote_peer).await;
        assert_eq!(sync.kind, WireKind::TerminalSyncRequest);
        let sync: v1::TerminalSyncRequest = sync
            .decode_message(WireKind::TerminalSyncRequest)
            .expect("full-sync request");
        assert_eq!(sync.known_revision, 5);
        assert_eq!(
            required_attachment_id(sync.attachment_id).expect("epoch ID"),
            remote_id
        );

        let inconsistent = decoded_message(
            WireKind::TerminalDelta,
            1,
            &valid_delta(remote_id, 4, 6, b"resume-gap"),
        );
        let mut state = bridge_state(target, local_id, session_id, Revision::new(5));
        assert!(matches!(
            accept_initial_remote_update(inconsistent, &mut state, None, 1)
                .expect("inconsistent resume is recoverable"),
            InitialRemoteUpdate::RequiresFull { .. }
        ));

        for kind in [
            DomainErrorKind::AuthorizationRevoked,
            DomainErrorKind::Unauthorized,
            DomainErrorKind::WireMajorMismatch,
            DomainErrorKind::MalformedFrame,
            DomainErrorKind::SessionNotFound,
            DomainErrorKind::LeaseLost,
        ] {
            assert!(!is_temporary_transport(kind), "{kind:?} is terminal");
        }
        for kind in [
            DomainErrorKind::TransportUnavailable,
            DomainErrorKind::AddressUnavailable,
            DomainErrorKind::DeadlineExceeded,
        ] {
            assert!(is_temporary_transport(kind), "{kind:?} reconnects");
        }

        let mut phase = EpochPhase::Active;
        let mut desired = DesiredViewPhase::Active;
        let mut local_sink = tokio::io::sink();
        let mut remote_sink = tokio::io::sink();
        let stale = decoded_message(
            WireKind::TerminalDelta,
            0,
            &valid_delta(attachment(0x75), 5, 6, b"stale"),
        );
        let error = process_epoch_remote_frame(
            stale,
            remote_id,
            &mut state,
            &mut local_sink,
            &mut remote_sink,
            EpochControl {
                phase: &mut phase,
                desired_phase: &mut desired,
                operation_timeout: Duration::from_secs(1),
            },
        )
        .await
        .expect_err("an old stream attachment ID cannot affect the current epoch");
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);

        for terminal in [
            decoded_message(
                WireKind::TerminalLeaseLost,
                0,
                &v1::TerminalLeaseLost {
                    attachment_id: Some(remote_id.into()),
                    generation: 9,
                },
            ),
            decoded_message(
                WireKind::TerminalSessionEnded,
                0,
                &v1::TerminalSessionEnded {
                    session_id: Some(session_id.into()),
                    attachment_id: Some(remote_id.into()),
                    reason: v1::TerminalSessionEndReason::NaturalExit as i32,
                    exit_code: 0,
                    signal: String::new(),
                },
            ),
            decode_one(
                &ServiceReply::error(
                    0,
                    &DaemonError::new(DomainErrorKind::AuthorizationRevoked, "revoked fixture"),
                )
                .bytes,
            ),
        ] {
            let mut phase = EpochPhase::Active;
            let end = process_epoch_remote_frame(
                terminal,
                remote_id,
                &mut state,
                &mut local_sink,
                &mut remote_sink,
                EpochControl {
                    phase: &mut phase,
                    desired_phase: &mut desired,
                    operation_timeout: Duration::from_secs(1),
                },
            )
            .await
            .expect("terminal event is projected safely");
            assert_eq!(end, Some(EpochEnd::Terminal));
        }
    }

    #[test]
    fn a_frozen_view_never_recreates_main_or_sends_a_pre_ack_reconnect_resize() {
        let target = device(0x81);
        let local_id = attachment(0x82);
        let session_id = session(0x83);
        let mut state = bridge_state(target, local_id, session_id, Revision::new(11));
        state.request.create_main = true;
        state.request.viewport = Some(v1::TerminalViewport {
            rows: 24,
            columns: 80,
        });
        state.latest_viewport = Some(v1::TerminalViewport {
            rows: 60,
            columns: 120,
        });
        let request = remote_attach_request(&state);
        assert!(!request.create_main);
        assert!(request.session_name.is_empty());
        assert_eq!(
            required_session_id(request.session_id).expect("frozen SessionId"),
            session_id
        );
        assert!(request.viewport.is_none());
        assert_eq!(request.known_revision, Some(11));
    }

    async fn run_first_host_epoch(
        stream: DuplexStream,
        session_id: SessionId,
        attachment_id: AttachmentId,
    ) -> FirstEpochEvidence {
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = FramedReader::fresh(reader);
        let attach = next_frame(&mut reader).await;
        let attach = attach
            .decode_message(WireKind::TerminalAttachRequest)
            .expect("first host attach request");
        write_message(
            &mut writer,
            WireKind::TerminalSnapshot,
            1,
            &valid_snapshot(session_id, attachment_id, 5),
        )
        .await;
        let acknowledgement = next_frame(&mut reader)
            .await
            .decode_message(WireKind::TerminalSnapshotApplied)
            .expect("first host acknowledgement");
        let resize = next_frame(&mut reader)
            .await
            .decode_message(WireKind::TerminalResize)
            .expect("first post-ack resize");
        let input = next_frame(&mut reader)
            .await
            .decode_message(WireKind::TerminalInput)
            .expect("first active input");
        writer.shutdown().await.expect("lose first transport epoch");
        FirstEpochEvidence {
            attach,
            acknowledgement,
            resize,
            input,
        }
    }

    async fn run_second_host_epoch(
        stream: DuplexStream,
        attachment_id: AttachmentId,
    ) -> SecondEpochEvidence {
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = FramedReader::fresh(reader);
        let attach = next_frame(&mut reader).await;
        let attach = attach
            .decode_message(WireKind::TerminalAttachRequest)
            .expect("second host attach request");
        write_message(
            &mut writer,
            WireKind::TerminalDelta,
            1,
            &valid_delta(attachment_id, 5, 6, b"resume"),
        )
        .await;
        let acknowledgement = next_frame(&mut reader)
            .await
            .decode_message(WireKind::TerminalSnapshotApplied)
            .expect("second host acknowledgement");
        let resize = next_frame(&mut reader)
            .await
            .decode_message(WireKind::TerminalResize)
            .expect("coalesced post-ack resize");
        let input = next_frame(&mut reader)
            .await
            .decode_message(WireKind::TerminalInput)
            .expect("second active input");
        let detach = next_frame(&mut reader)
            .await
            .decode_message(WireKind::TerminalDetach)
            .expect("explicit current-epoch detach");
        SecondEpochEvidence {
            attach,
            acknowledgement,
            resize,
            input,
            detach,
        }
    }

    struct TakeoverEpochEvidence {
        epoch: usize,
        lease_requests: usize,
        issued_lease: Option<OperationLease>,
        takeover: v1::SessionTakeoverRequest,
        response_frame: Vec<u8>,
        response_payload: Vec<u8>,
    }

    struct FlushObservedStream {
        inner: DuplexStream,
        flushed: mpsc::Sender<()>,
    }

    impl AsyncRead for FlushObservedStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_read(context, buffer)
        }
    }

    impl AsyncWrite for FlushObservedStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.inner).poll_write(context, bytes)
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            match Pin::new(&mut self.inner).poll_flush(context) {
                Poll::Ready(Ok(())) => {
                    self.flushed
                        .try_send(())
                        .expect("test flush observer stays within its fixed response bound");
                    Poll::Ready(Ok(()))
                }
                result => result,
            }
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_shutdown(context)
        }
    }

    fn start_relayed_takeover_epoch(
        epoch: usize,
        drop_mutation_response: bool,
        server: SessionWireServer,
        context: SessionRequestContext,
        evidence: mpsc::Sender<TakeoverEpochEvidence>,
        detached: Option<oneshot::Sender<()>>,
    ) -> (
        DuplexStream,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<Result<(), DaemonError>>,
    ) {
        let (bridge_stream, relay_bridge) = duplex(64 * 1024);
        let (relay_host, server_stream) = duplex(64 * 1024);
        let (flushed, flushes) = mpsc::channel(MAX_PENDING_CONTROL_REQUESTS);
        let server_stream = FlushObservedStream {
            inner: server_stream,
            flushed,
        };
        let server_task = tokio::spawn(async move {
            let result = server
                .handle_remote_stream(
                    server_stream,
                    context,
                    SessionWireLimits::default(),
                    Instant::now() + Duration::from_secs(5),
                )
                .await;
            if let Some(detached) = detached {
                let _ = detached.send(());
            }
            result
        });
        let relay_task = tokio::spawn(async move {
            relay_takeover_epoch(
                epoch,
                drop_mutation_response,
                relay_bridge,
                relay_host,
                flushes,
                evidence,
            )
            .await;
        });
        (bridge_stream, relay_task, server_task)
    }

    async fn relay_takeover_epoch(
        epoch: usize,
        drop_mutation_response: bool,
        relay_bridge: DuplexStream,
        relay_host: DuplexStream,
        mut host_flushes: mpsc::Receiver<()>,
        evidence: mpsc::Sender<TakeoverEpochEvidence>,
    ) {
        let (bridge_reader, mut bridge_writer) = tokio::io::split(relay_bridge);
        let (host_reader, mut host_writer) = tokio::io::split(relay_host);
        let mut bridge_reader = FramedReader::fresh(bridge_reader);
        let mut host_reader = FramedReader::fresh(host_reader);
        let mut lease_requests = 0;
        let mut issued_lease = None;
        let mut takeover = None;

        loop {
            tokio::select! {
                request = bridge_reader.next() => {
                    let Some(request) = request.expect("decode relayed bridge request") else {
                        let _ = host_writer.shutdown().await;
                        return;
                    };
                    if request.kind == WireKind::SessionOperationLeaseRequest {
                        lease_requests += 1;
                    }
                    if request.kind == WireKind::SessionTakeoverRequest {
                        takeover = Some(
                            (
                                request.request_id,
                                request
                                    .decode_message(WireKind::SessionTakeoverRequest)
                                    .expect("decode relayed takeover request"),
                            ),
                        );
                    }
                    host_writer
                        .write_all(&decoded_frame_bytes(&request))
                        .await
                        .expect("forward bridge request to real Session wire server");
                }
                response = host_reader.next() => {
                    let Some(response) = response.expect("decode relayed host response") else {
                        let _ = bridge_writer.shutdown().await;
                        return;
                    };
                    host_flushes
                        .recv()
                        .await
                        .expect("real Session wire server flushes each captured response");
                    if response.kind == WireKind::SessionOperationLeaseResponse {
                        issued_lease = Some(
                            response
                                .decode_message::<v1::SessionOperationLeaseResponse>(
                                    WireKind::SessionOperationLeaseResponse,
                                )
                                .expect("decode relayed operation lease response")
                                .lease
                                .expect("relayed operation lease response contains a lease")
                                .try_into()
                                .expect("relayed operation lease is valid"),
                        );
                    }
                    let is_mutation = response.kind == WireKind::SessionMutateResponse;
                    let response_frame = decoded_frame_bytes(&response);
                    if !is_mutation || !drop_mutation_response {
                        bridge_writer
                            .write_all(&response_frame)
                            .await
                            .expect("forward host response to attachment bridge");
                    }
                    if is_mutation {
                        let (takeover_request_id, takeover) = takeover
                            .take()
                            .expect("takeover response follows its request");
                        assert_eq!(response.request_id, takeover_request_id);
                        evidence
                            .send(TakeoverEpochEvidence {
                                epoch,
                                lease_requests,
                                issued_lease,
                                takeover,
                                response_frame,
                                response_payload: response.payload,
                            })
                            .await
                            .expect("takeover evidence receiver remains live");
                        if drop_mutation_response {
                            let _ = bridge_writer.shutdown().await;
                            let _ = host_writer.shutdown().await;
                            return;
                        }
                    }
                }
            }
        }
    }

    fn decoded_frame_bytes(frame: &DecodedFrame) -> Vec<u8> {
        zterm_proto::encode_payload(
            frame.kind,
            frame.request_id,
            frame.deadline_ms,
            frame.payload.clone(),
        )
        .expect("re-encode complete relayed frame")
    }

    fn decoded_first_frame<Message: prost::Message>(
        kind: WireKind,
        request_id: u64,
        message: &Message,
    ) -> FirstFrame {
        let bytes = encode_message(kind, request_id, 5_000, message)
            .expect("encode local bridge first frame");
        let mut decoder = FrameDecoder::new();
        let mut frames = VecDeque::from(decoder.feed(&bytes).expect("decode local first frame"));
        let frame = frames.pop_front().expect("one local first frame");
        assert!(frames.is_empty());
        FirstFrame {
            frame,
            decoder,
            queued: frames,
        }
    }

    async fn apply_next_bridge_initial(client: &mut LocalAttachmentClient) -> Revision {
        loop {
            match client
                .read_event(Duration::from_secs(2))
                .await
                .expect("bridge initial synchronization event")
            {
                LocalAttachmentEvent::Snapshot(snapshot) => {
                    let revision = Revision::new(snapshot.revision);
                    client
                        .snapshot_applied(revision)
                        .await
                        .expect("acknowledge bridge snapshot");
                    return revision;
                }
                LocalAttachmentEvent::Delta(delta) => {
                    let revision = Revision::new(delta.to_revision);
                    client
                        .snapshot_applied(revision)
                        .await
                        .expect("acknowledge bridge resume delta");
                    return revision;
                }
                LocalAttachmentEvent::TransportState(_)
                | LocalAttachmentEvent::SyncRequired(_)
                | LocalAttachmentEvent::ConnectionStatus(_) => {}
                event => panic!("unexpected bridge synchronization event: {event:?}"),
            }
        }
    }

    async fn await_takeover_epoch_task(task: tokio::task::JoinHandle<()>, label: &str) {
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap_or_else(|_| panic!("{label} did not finish"))
            .unwrap_or_else(|error| panic!("{label} panicked: {error}"));
    }

    async fn await_takeover_server_task(
        task: tokio::task::JoinHandle<Result<(), DaemonError>>,
        label: &str,
    ) {
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap_or_else(|_| panic!("{label} did not finish"))
            .unwrap_or_else(|error| panic!("{label} panicked: {error}"))
            .unwrap_or_else(|error| panic!("{label} failed: {error}"));
    }

    fn takeover_authorization(
        remote: DeviceId,
        accepted_generation: AuthGeneration,
    ) -> AuthorizationRegistry {
        let authorization = AuthorizationRegistry::new();
        authorization
            .preload(vec![DeviceAuthorization {
                device_id: remote,
                display_name: DeviceDisplayName::new("takeover response-loss controller")
                    .expect("test display name"),
                status: AuthorizationStatus::Authorized,
                generation: accepted_generation,
                paired_at_unix: 1,
                revoked_at_unix: None,
                last_seen_at_unix: None,
            }])
            .expect("preload takeover authorization");
        authorization
    }

    fn takeover_session_service(
        own_device_id: DeviceId,
        working_directory: PathBuf,
        spawn_count: Arc<std::sync::atomic::AtomicUsize>,
    ) -> SessionService {
        let cat = [Path::new("/bin/cat"), Path::new("/usr/bin/cat")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX cat fixture")
            .to_path_buf();
        SessionService::with_spawner(
            own_device_id,
            ResourceLimits::default(),
            move |size, requested| {
                spawn_count.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                let cwd = requested.unwrap_or(&working_directory).to_path_buf();
                let session = PtyHost::new()
                    .spawn(
                        ExplicitPtyCommand::new(&cat, &cwd),
                        PtySize::new(size.rows, size.columns),
                    )
                    .map_err(|_| {
                        DaemonError::new(
                            DomainErrorKind::StoreUnavailable,
                            "task-private PTY spawn failed",
                        )
                    })?;
                Ok((session, cwd))
            },
        )
    }

    async fn start_pending_view(
        client: RemoteAttachmentClient,
        target: DeviceId,
        local_id: AttachmentId,
    ) -> (UnixStream, tokio::task::JoinHandle<Result<(), DaemonError>>) {
        start_view_with_limits(client, target, local_id, SessionWireLimits::default()).await
    }

    async fn start_view_with_limits(
        client: RemoteAttachmentClient,
        target: DeviceId,
        local_id: AttachmentId,
        limits: SessionWireLimits,
    ) -> (UnixStream, tokio::task::JoinHandle<Result<(), DaemonError>>) {
        let (mut local_cli, mut local_daemon) = UnixStream::pair().expect("Unix pending-view pair");
        write_message(
            &mut local_cli,
            WireKind::TerminalAttachRequest,
            1,
            &attach_main(target),
        )
        .await;
        let first = read_first(&mut local_daemon)
            .await
            .expect("pending view first frame");
        let task = tokio::spawn(async move {
            client
                .serve(
                    target,
                    local_id,
                    local_daemon,
                    first,
                    limits,
                    Instant::now() + Duration::from_secs(2),
                )
                .await
        });
        (local_cli, task)
    }

    fn bridge_state(
        target: DeviceId,
        local_view_id: AttachmentId,
        session_id: SessionId,
        known_revision: Revision,
    ) -> BridgeState {
        BridgeState {
            request: attach_main(target),
            request_id: 1,
            target,
            local_view_id,
            resume_view_id: ResumeViewId::from_array([0xa1; 16]),
            frozen_session_id: Some(session_id),
            known_revision: Some(known_revision),
            latest_viewport: None,
            force_full: false,
            ever_active: true,
            pending_control: BTreeMap::new(),
        }
    }

    fn attach_main(target: DeviceId) -> v1::TerminalAttachRequest {
        v1::TerminalAttachRequest {
            target: Some(v1::TargetSelector {
                target: Some(v1::target_selector::Target::Device(target.into())),
            }),
            session_id: None,
            takeover: false,
            session_name: String::new(),
            create_main: true,
            viewport: Some(v1::TerminalViewport {
                rows: 24,
                columns: 80,
            }),
            resume_view_id: None,
            known_revision: None,
        }
    }

    fn device_target(target: DeviceId) -> v1::TargetSelector {
        v1::TargetSelector {
            target: Some(v1::target_selector::Target::Device(target.into())),
        }
    }

    fn fixture_operation_id() -> OperationId {
        OperationId {
            lease: OperationLease {
                daemon_incarnation: DaemonIncarnation::from_array([0x91; 16]),
                ordinal: 1,
            },
            sequence: 1,
        }
    }

    fn service_error_kind(frame: &DecodedFrame) -> DomainErrorKind {
        let error: v1::ServiceError = frame
            .decode_message(WireKind::ServiceErrorResponse)
            .expect("typed local service error");
        DomainErrorKind::from_code(&error.code).expect("stable service error code")
    }

    fn service_error_message(frame: &DecodedFrame) -> String {
        let error: v1::ServiceError = frame
            .decode_message(WireKind::ServiceErrorResponse)
            .expect("typed local service error");
        error.message
    }

    fn valid_snapshot(
        session_id: SessionId,
        attachment_id: AttachmentId,
        revision: u64,
    ) -> v1::TerminalSnapshot {
        v1::TerminalSnapshot {
            session_id: Some(session_id.into()),
            attachment_id: Some(attachment_id.into()),
            revision,
            rows: 24,
            columns: 80,
            screen_ansi: b"snapshot".to_vec(),
            recent_history_ansi: Vec::new(),
            active_screen: v1::TerminalActiveScreen::Main as i32,
            modes: Some(v1::TerminalModes::default()),
        }
    }

    fn valid_delta(
        attachment_id: AttachmentId,
        from_revision: u64,
        to_revision: u64,
        ansi: &[u8],
    ) -> v1::TerminalDelta {
        v1::TerminalDelta {
            from_revision,
            to_revision,
            ansi: ansi.to_vec(),
            rows: 24,
            columns: 80,
            active_screen: v1::TerminalActiveScreen::Main as i32,
            modes: Some(v1::TerminalModes::default()),
            attachment_id: Some(attachment_id.into()),
        }
    }

    async fn expect_transport_state<Reader>(
        reader: &mut FramedReader<Reader>,
        attachment_id: AttachmentId,
        expected: v1::TerminalTransportState,
    ) where
        Reader: AsyncRead + Unpin,
    {
        assert_transport_state(
            next_non_status_frame(reader, attachment_id).await,
            attachment_id,
            expected,
        );
    }

    fn assert_transport_state(
        frame: DecodedFrame,
        attachment_id: AttachmentId,
        expected: v1::TerminalTransportState,
    ) {
        assert_eq!(frame.kind, WireKind::TerminalTransportStateEvent);
        let state: v1::TerminalTransportStateEvent = frame
            .decode_message(WireKind::TerminalTransportStateEvent)
            .expect("transport-state event");
        assert_eq!(
            required_attachment_id(state.attachment_id).expect("transport-state attachment ID"),
            attachment_id
        );
        assert_eq!(
            v1::TerminalTransportState::try_from(state.state).expect("known transport state"),
            expected
        );
    }

    async fn expect_paused_transport_state<Reader>(
        reader: &mut FramedReader<Reader>,
        attachment_id: AttachmentId,
        expected: v1::TerminalTransportState,
    ) where
        Reader: AsyncRead + Unpin,
    {
        loop {
            let frame = next_paused_frame(reader).await;
            if frame.kind == WireKind::TerminalConnectionStatusEvent {
                assert_connection_status(frame, attachment_id);
                continue;
            }
            assert_transport_state(frame, attachment_id, expected);
            return;
        }
    }

    async fn expect_paused_connection_status<Reader>(
        reader: &mut FramedReader<Reader>,
        attachment_id: AttachmentId,
    ) -> v1::TerminalConnectionStatusEvent
    where
        Reader: AsyncRead + Unpin,
    {
        assert_connection_status(next_paused_frame(reader).await, attachment_id)
    }

    fn assert_connection_status(
        frame: DecodedFrame,
        attachment_id: AttachmentId,
    ) -> v1::TerminalConnectionStatusEvent {
        assert_eq!(frame.kind, WireKind::TerminalConnectionStatusEvent);
        let status: v1::TerminalConnectionStatusEvent = frame
            .decode_message(WireKind::TerminalConnectionStatusEvent)
            .expect("connection-status event");
        assert_eq!(
            required_attachment_id(status.attachment_id.clone())
                .expect("connection-status attachment ID"),
            attachment_id
        );
        v1::TerminalConnectionPath::try_from(status.path).expect("known connection path");
        status
    }

    async fn next_non_status_frame<Reader>(
        reader: &mut FramedReader<Reader>,
        attachment_id: AttachmentId,
    ) -> DecodedFrame
    where
        Reader: AsyncRead + Unpin,
    {
        loop {
            let frame = next_frame(reader).await;
            if frame.kind == WireKind::TerminalConnectionStatusEvent {
                assert_connection_status(frame, attachment_id);
            } else {
                return frame;
            }
        }
    }

    async fn next_paused_frame<Reader>(reader: &mut FramedReader<Reader>) -> DecodedFrame
    where
        Reader: AsyncRead + Unpin,
    {
        reader
            .next()
            .await
            .expect("valid frame under paused time")
            .expect("stream remains open under paused time")
    }

    async fn next_frame<Reader>(reader: &mut FramedReader<Reader>) -> DecodedFrame
    where
        Reader: AsyncRead + Unpin,
    {
        tokio::time::timeout(Duration::from_secs(2), reader.next())
            .await
            .expect("frame deadline")
            .expect("valid frame")
            .expect("stream remains open")
    }

    async fn write_message<Writer, Message>(
        writer: &mut Writer,
        kind: WireKind,
        request_id: u64,
        message: &Message,
    ) where
        Writer: AsyncWrite + Unpin,
        Message: prost::Message,
    {
        let bytes = encode_message(kind, request_id, 0, message).expect("bounded fixture frame");
        writer.write_all(&bytes).await.expect("write fixture frame");
        writer.flush().await.expect("flush fixture frame");
    }

    fn decoded_message<Message>(kind: WireKind, request_id: u64, message: &Message) -> DecodedFrame
    where
        Message: prost::Message,
    {
        decode_one(&encode_message(kind, request_id, 0, message).expect("bounded fixture frame"))
    }

    fn decode_one(bytes: &[u8]) -> DecodedFrame {
        let mut decoder = FrameDecoder::new();
        let mut frames = decoder.feed(bytes).expect("decode fixture frame");
        decoder.finish().expect("complete fixture frame");
        assert_eq!(frames.len(), 1);
        frames.remove(0)
    }

    fn device(byte: u8) -> DeviceId {
        DeviceId::from_array([byte; DeviceId::LENGTH])
    }

    fn session(byte: u8) -> SessionId {
        SessionId::from_array([byte; SessionId::LENGTH])
    }

    fn attachment(byte: u8) -> AttachmentId {
        AttachmentId::from_array([byte; AttachmentId::LENGTH])
    }
}
