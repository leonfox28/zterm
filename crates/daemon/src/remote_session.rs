//! Daemon-owned outbound Session unary forwarding.
//!
//! This module is the only outbound owner of connection demand, service-stream
//! attempts, strict response framing, and ambiguity retry classification. It
//! forwards one caller-preencoded Session frame unchanged and never owns
//! Session, replay, endpoint, route, or persistent-store state.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroize;
use zterm_core::{AttachmentId, DeviceId, DomainErrorKind, OperationLease, Revision, SessionName};
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, v1};

use crate::connection_broker::{ConnectionBroker, ConnectionDemand, StreamPurpose};
use crate::device_directory::{DeviceDirectory, ResolvedSessionTarget};
use crate::error::DaemonError;
use crate::remote_attachment::RemoteAttachmentClient;
use crate::service::protocol_error;
use crate::session::SessionSummary;
use crate::session_wire::{FirstFrame, SessionWireLimits};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
const REMOTE_SESSION_FAILURE_DETAIL: &str = "remote Session request failed";

/// Daemon composition for exact resolution and outbound Session unary calls.
#[derive(Clone)]
pub(crate) struct RemoteSessionService {
    own_device_id: DeviceId,
    directory: DeviceDirectory,
    client: Option<RemoteUnaryClient>,
    attachments: Option<RemoteAttachmentClient>,
}

impl RemoteSessionService {
    /// Composes production forwarding around the existing broker owner.
    pub(crate) fn production(
        own_device_id: DeviceId,
        directory: DeviceDirectory,
        broker: ConnectionBroker,
    ) -> Self {
        let unary_broker = broker.clone();
        Self {
            own_device_id,
            directory,
            client: Some(RemoteUnaryClient::new(Arc::new(
                BrokerRemoteUnaryTransport {
                    broker: unary_broker,
                },
            ))),
            attachments: Some(RemoteAttachmentClient::production(broker)),
        }
    }

    /// Composes exact resolution without preparing or binding any network owner.
    pub(crate) const fn local_only(own_device_id: DeviceId, directory: DeviceDirectory) -> Self {
        Self {
            own_device_id,
            directory,
            client: None,
            attachments: None,
        }
    }

    pub(crate) fn resolve(
        &self,
        selector: &str,
        deadline: Instant,
    ) -> Result<ResolvedSessionTarget, DaemonError> {
        if selector
            .parse::<DeviceId>()
            .is_ok_and(|device_id| device_id == self.own_device_id)
        {
            return Err(DaemonError::new(
                DomainErrorKind::InvalidTargetSelector,
                "the local device must use the reserved local target",
            ));
        }
        let target = self.directory.resolve_session_target(selector, deadline)?;
        if target.device_id() == Some(self.own_device_id) {
            return Err(DaemonError::new(
                DomainErrorKind::InvalidTargetSelector,
                "the local device must use the reserved local target",
            ));
        }
        Ok(target)
    }

    pub(crate) async fn forward_preencoded(
        &self,
        target: DeviceId,
        request_id: u64,
        request: &[u8],
        deadline: Instant,
    ) -> Result<DecodedFrame, DaemonError> {
        if target == self.own_device_id {
            return Err(DaemonError::new(
                DomainErrorKind::InvalidTargetSelector,
                "the local device must use the reserved local target",
            ));
        }
        let contract = RequestContract::inspect(request, target)?;
        contract.require_request_id(request_id)?;
        let directory = self.directory.clone();
        run_blocking_until(deadline, move || {
            directory
                .require_outbound_device(target, deadline)
                .map(|_| ())
        })
        .await?;
        let Some(client) = &self.client else {
            return Err(DaemonError::new(
                DomainErrorKind::TransportUnavailable,
                "remote Session transport is unavailable in this local-only daemon",
            ));
        };
        client
            .execute_validated(target, request, deadline, contract)
            .await
    }

    pub(crate) async fn bridge_attachment<Stream>(
        &self,
        target: DeviceId,
        local_view_id: AttachmentId,
        mut stream: Stream,
        first: FirstFrame,
        limits: SessionWireLimits,
        deadline: Instant,
    ) -> Result<(), DaemonError>
    where
        Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let validation = if target == self.own_device_id {
            Err(DaemonError::new(
                DomainErrorKind::InvalidTargetSelector,
                "the local device must use the reserved local target",
            ))
        } else {
            let directory = self.directory.clone();
            run_blocking_until(deadline, move || {
                directory
                    .require_outbound_device(target, deadline)
                    .map(|_| ())
            })
            .await
        };
        if let Err(error) = validation {
            write_local_attachment_error_best_effort(
                &mut stream,
                first.frame.request_id,
                &error,
                deadline,
            )
            .await;
            return Err(error);
        }
        let Some(client) = self.attachments.as_ref() else {
            let error = DaemonError::new(
                DomainErrorKind::TransportUnavailable,
                "remote attachment transport is unavailable in this local-only daemon",
            );
            write_local_attachment_error_best_effort(
                &mut stream,
                first.frame.request_id,
                &error,
                deadline,
            )
            .await;
            return Err(error);
        };
        client
            .serve(target, local_view_id, stream, first, limits, deadline)
            .await
    }
}

/// One logical outbound unary client. It acquires one demand and keeps it for
/// both possible service-stream attempts.
#[derive(Clone)]
struct RemoteUnaryClient {
    transport: Arc<dyn RemoteUnaryTransport>,
}

impl RemoteUnaryClient {
    fn new(transport: Arc<dyn RemoteUnaryTransport>) -> Self {
        Self { transport }
    }

    #[cfg(test)]
    async fn execute_preencoded(
        &self,
        target: DeviceId,
        request_id: u64,
        request: &[u8],
        deadline: Instant,
    ) -> Result<DecodedFrame, DaemonError> {
        let contract = RequestContract::inspect(request, target)?;
        contract.require_request_id(request_id)?;
        self.execute_validated(target, request, deadline, contract)
            .await
    }

    async fn execute_validated(
        &self,
        target: DeviceId,
        request: &[u8],
        deadline: Instant,
        contract: RequestContract,
    ) -> Result<DecodedFrame, DaemonError> {
        let mut demand = self.transport.demand(target, deadline).await?;

        let first = exchange_and_validate(&mut *demand, request, deadline, contract).await;
        match first {
            Ok(response) => Ok(response),
            Err(RemoteAttemptError::PreWrite(error)) => Err(error),
            Err(RemoteAttemptError::PostWrite(first_error)) => match contract.retry_class {
                RetryClass::StatefulControl => Err(first_error),
                RetryClass::Safe | RetryClass::Mutation => {
                    let second =
                        exchange_and_validate(&mut *demand, request, deadline, contract).await;
                    match second {
                        Ok(response) => Ok(response),
                        Err(_) if contract.retry_class == RetryClass::Mutation => {
                            Err(outcome_unknown())
                        }
                        Err(
                            RemoteAttemptError::PreWrite(error)
                            | RemoteAttemptError::PostWrite(error),
                        ) => Err(error),
                    }
                }
            },
        }
    }
}

async fn exchange_and_validate(
    demand: &mut dyn RemoteUnaryDemand,
    request: &[u8],
    deadline: Instant,
    contract: RequestContract,
) -> Result<DecodedFrame, RemoteAttemptError> {
    let response = demand.exchange(request, deadline).await?;
    contract
        .validate_response(response)
        .map_err(RemoteAttemptError::PostWrite)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetryClass {
    Safe,
    StatefulControl,
    Mutation,
}

#[derive(Clone, Copy, Debug)]
struct RequestContract {
    request_id: u64,
    response_kind: WireKind,
    retry_class: RetryClass,
}

impl RequestContract {
    fn inspect(request: &[u8], target: DeviceId) -> Result<Self, DaemonError> {
        let frame = decode_exact_frame(request)?;
        let (wire_target, response_kind, retry_class) = match frame.kind {
            WireKind::SessionListRequest => {
                let message: v1::SessionListRequest = frame
                    .decode_message(WireKind::SessionListRequest)
                    .map_err(protocol_error)?;
                (
                    message.target,
                    WireKind::SessionListResponse,
                    RetryClass::Safe,
                )
            }
            WireKind::SessionOperationLeaseRequest => {
                let message: v1::SessionOperationLeaseRequest = frame
                    .decode_message(WireKind::SessionOperationLeaseRequest)
                    .map_err(protocol_error)?;
                (
                    message.target,
                    WireKind::SessionOperationLeaseResponse,
                    RetryClass::StatefulControl,
                )
            }
            WireKind::SessionCreateRequest => {
                let message: v1::SessionCreateRequest = frame
                    .decode_message(WireKind::SessionCreateRequest)
                    .map_err(protocol_error)?;
                (
                    message.target,
                    WireKind::SessionMutateResponse,
                    RetryClass::Mutation,
                )
            }
            WireKind::SessionRenameRequest => {
                let message: v1::SessionRenameRequest = frame
                    .decode_message(WireKind::SessionRenameRequest)
                    .map_err(protocol_error)?;
                (
                    message.target,
                    WireKind::SessionMutateResponse,
                    RetryClass::Mutation,
                )
            }
            WireKind::SessionCloseRequest => {
                let message: v1::SessionCloseRequest = frame
                    .decode_message(WireKind::SessionCloseRequest)
                    .map_err(protocol_error)?;
                (
                    message.target,
                    WireKind::SessionMutateResponse,
                    RetryClass::Mutation,
                )
            }
            WireKind::SessionTakeoverRequest => {
                let message: v1::SessionTakeoverRequest = frame
                    .decode_message(WireKind::SessionTakeoverRequest)
                    .map_err(protocol_error)?;
                (
                    message.target,
                    WireKind::SessionMutateResponse,
                    RetryClass::Mutation,
                )
            }
            _ => {
                return Err(DaemonError::new(
                    DomainErrorKind::MalformedFrame,
                    "local remote-Session envelope contains a non-unary Session kind",
                ));
            }
        };
        require_exact_remote_target(wire_target, target)?;
        Ok(Self {
            request_id: frame.request_id,
            response_kind,
            retry_class,
        })
    }

    fn require_request_id(self, request_id: u64) -> Result<(), DaemonError> {
        if self.request_id == request_id {
            Ok(())
        } else {
            Err(DaemonError::new(
                DomainErrorKind::MalformedFrame,
                "local forwarding envelope request_id differs from its inner Session request",
            ))
        }
    }

    fn validate_response(self, response: DecodedFrame) -> Result<DecodedFrame, DaemonError> {
        match validate_session_unary_response(&response, self.request_id, self.response_kind)? {
            SessionUnaryResponseStatus::Expected => Ok(response),
            SessionUnaryResponseStatus::ServiceError(error) => {
                Ok(content_free_service_error_frame(response, &error))
            }
        }
    }
}

pub(crate) enum SessionUnaryResponseStatus {
    Expected,
    ServiceError(DaemonError),
}

pub(crate) fn validate_session_unary_response(
    frame: &DecodedFrame,
    expected_request_id: u64,
    expected_kind: WireKind,
) -> Result<SessionUnaryResponseStatus, DaemonError> {
    if frame.request_id != expected_request_id {
        return Err(DaemonError::new(
            DomainErrorKind::MalformedFrame,
            "remote Session response request_id does not match its request",
        ));
    }
    if frame.kind == WireKind::ServiceErrorResponse {
        return decode_session_service_error(frame).map(SessionUnaryResponseStatus::ServiceError);
    }
    if frame.kind != expected_kind {
        return Err(DaemonError::new(
            DomainErrorKind::MalformedFrame,
            format!(
                "remote Session response kind {:?} does not match expected {:?}",
                frame.kind, expected_kind
            ),
        ));
    }
    validate_session_unary_response_payload(frame, expected_kind)?;
    Ok(SessionUnaryResponseStatus::Expected)
}

pub(crate) fn decode_session_service_error(
    frame: &DecodedFrame,
) -> Result<DaemonError, DaemonError> {
    let mut response: v1::ServiceError = frame
        .decode_message(WireKind::ServiceErrorResponse)
        .map_err(protocol_error)?;
    let kind = DomainErrorKind::from_code(&response.code);
    response.message.zeroize();
    let kind = kind.ok_or_else(|| {
        DaemonError::new(
            DomainErrorKind::MalformedFrame,
            "remote Session response used an unknown domain error code",
        )
    })?;
    Ok(DaemonError::new(kind, REMOTE_SESSION_FAILURE_DETAIL))
}

fn content_free_service_error_frame(frame: DecodedFrame, error: &DaemonError) -> DecodedFrame {
    let response = v1::ServiceError {
        code: error.kind().code().to_owned(),
        message: error.detail().to_owned(),
    };
    DecodedFrame {
        kind: WireKind::ServiceErrorResponse,
        request_id: frame.request_id,
        deadline_ms: 0,
        payload: response.encode_to_vec(),
    }
}

fn validate_session_unary_response_payload(
    frame: &DecodedFrame,
    expected_kind: WireKind,
) -> Result<(), DaemonError> {
    match expected_kind {
        WireKind::SessionListResponse => {
            let response: v1::SessionListResponse = frame
                .decode_message(expected_kind)
                .map_err(protocol_error)?;
            for summary in response.sessions {
                session_summary_from_wire(summary)?;
            }
            Ok(())
        }
        WireKind::SessionOperationLeaseResponse => {
            let response: v1::SessionOperationLeaseResponse = frame
                .decode_message(expected_kind)
                .map_err(protocol_error)?;
            let _: OperationLease = response
                .lease
                .ok_or_else(|| malformed("operation lease response omitted lease"))?
                .try_into()
                .map_err(protocol_error)?;
            Ok(())
        }
        WireKind::SessionMutateResponse => {
            let response: v1::SessionMutateResponse = frame
                .decode_message(expected_kind)
                .map_err(protocol_error)?;
            session_summary_from_wire(
                response
                    .session
                    .ok_or_else(|| malformed("session mutation response omitted session"))?,
            )?;
            Ok(())
        }
        _ => Err(malformed(
            "remote Session client expected a non-unary response kind",
        )),
    }
}

pub(crate) fn session_summary_from_wire(
    summary: v1::SessionSummary,
) -> Result<SessionSummary, DaemonError> {
    let session_id = summary
        .session_id
        .ok_or_else(|| malformed("session summary omitted session_id"))?
        .try_into()
        .map_err(protocol_error)?;
    let name = SessionName::new(summary.name).map_err(|error| malformed(error.to_string()))?;
    let viewport = summary
        .viewport
        .ok_or_else(|| malformed("session summary omitted viewport"))?
        .try_into()
        .map_err(protocol_error)?;
    Ok(SessionSummary {
        session_id,
        name,
        revision: Revision::new(summary.revision),
        has_controller: summary.has_controller,
        working_directory: PathBuf::from(summary.working_directory),
        viewport,
    })
}

trait RemoteUnaryTransport: Send + Sync {
    fn demand<'a>(
        &'a self,
        target: DeviceId,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Box<dyn RemoteUnaryDemand>, DaemonError>>;
}

trait RemoteUnaryDemand: Send {
    fn exchange<'a>(
        &'a mut self,
        request: &'a [u8],
        deadline: Instant,
    ) -> BoxFuture<'a, Result<DecodedFrame, RemoteAttemptError>>;
}

struct BrokerRemoteUnaryTransport {
    broker: ConnectionBroker,
}

impl RemoteUnaryTransport for BrokerRemoteUnaryTransport {
    fn demand<'a>(
        &'a self,
        target: DeviceId,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Box<dyn RemoteUnaryDemand>, DaemonError>> {
        Box::pin(async move {
            let demand = self.broker.demand(target, deadline).await?;
            Ok(Box::new(BrokerRemoteUnaryDemand { demand }) as Box<dyn RemoteUnaryDemand>)
        })
    }
}

struct BrokerRemoteUnaryDemand {
    demand: ConnectionDemand,
}

impl RemoteUnaryDemand for BrokerRemoteUnaryDemand {
    fn exchange<'a>(
        &'a mut self,
        request: &'a [u8],
        deadline: Instant,
    ) -> BoxFuture<'a, Result<DecodedFrame, RemoteAttemptError>> {
        Box::pin(async move {
            let mut stream = self
                .demand
                .open_bi(StreamPurpose::Service, deadline)
                .await
                .map_err(RemoteAttemptError::PreWrite)?;
            timeout_until(deadline, stream.send.write_all(request))
                .await
                .map_err(RemoteAttemptError::PostWrite)?
                .map_err(|_| {
                    RemoteAttemptError::PostWrite(transport_unavailable(
                        "remote Session request write failed",
                    ))
                })?;
            stream.send.finish().map_err(|_| {
                RemoteAttemptError::PostWrite(transport_unavailable(
                    "remote Session request finish failed",
                ))
            })?;
            read_exact_response(&mut stream.recv, deadline).await
        })
    }
}

enum RemoteAttemptError {
    PreWrite(DaemonError),
    PostWrite(DaemonError),
}

async fn read_exact_response<Reader>(
    reader: &mut Reader,
    deadline: Instant,
) -> Result<DecodedFrame, RemoteAttemptError>
where
    Reader: AsyncRead + Unpin,
{
    timeout_until(deadline, async {
        let mut decoder = FrameDecoder::new();
        let mut completed = None;
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = reader.read(&mut buffer).await.map_err(|_| {
                RemoteAttemptError::PostWrite(transport_unavailable(
                    "remote Session response read failed",
                ))
            })?;
            if read == 0 {
                decoder
                    .finish()
                    .map_err(|error| RemoteAttemptError::PostWrite(protocol_error(error)))?;
                return completed.ok_or_else(|| {
                    RemoteAttemptError::PostWrite(transport_unavailable(
                        "remote Session stream ended without a response",
                    ))
                });
            }
            let frames = decoder
                .feed(&buffer[..read])
                .map_err(|error| RemoteAttemptError::PostWrite(protocol_error(error)))?;
            if frames.len() > 1 || (completed.is_some() && !frames.is_empty()) {
                return Err(RemoteAttemptError::PostWrite(DaemonError::new(
                    DomainErrorKind::MalformedFrame,
                    "remote Session stream returned more than one response",
                )));
            }
            if let Some(frame) = frames.into_iter().next() {
                completed = Some(frame);
            }
        }
    })
    .await
    .map_err(RemoteAttemptError::PostWrite)?
}

fn decode_exact_frame(bytes: &[u8]) -> Result<DecodedFrame, DaemonError> {
    let mut decoder = FrameDecoder::new();
    let mut frames = decoder.feed(bytes).map_err(protocol_error)?;
    decoder.finish().map_err(protocol_error)?;
    if frames.len() != 1 {
        return Err(DaemonError::new(
            DomainErrorKind::MalformedFrame,
            "remote Session envelope must contain exactly one complete frame",
        ));
    }
    frames.pop().ok_or_else(|| {
        DaemonError::new(
            DomainErrorKind::MalformedFrame,
            "remote Session envelope omitted its inner frame",
        )
    })
}

fn require_exact_remote_target(
    target: Option<v1::TargetSelector>,
    expected: DeviceId,
) -> Result<(), DaemonError> {
    let Some(v1::target_selector::Target::Device(device)) = target.and_then(|target| target.target)
    else {
        return Err(DaemonError::new(
            DomainErrorKind::MalformedFrame,
            "remote Session request omitted its exact device target",
        ));
    };
    let actual: DeviceId = device.try_into().map_err(protocol_error)?;
    if actual != expected {
        return Err(DaemonError::new(
            DomainErrorKind::MalformedFrame,
            "remote Session envelope target differs from its inner request target",
        ));
    }
    Ok(())
}

async fn run_blocking_until<T>(
    deadline: Instant,
    operation: impl FnOnce() -> Result<T, DaemonError> + Send + 'static,
) -> Result<T, DaemonError>
where
    T: Send + 'static,
{
    timeout_until(deadline, tokio::task::spawn_blocking(operation))
        .await?
        .map_err(|error| {
            DaemonError::new(
                DomainErrorKind::Cancelled,
                format!("remote Session resolver worker ended unexpectedly: {error}"),
            )
        })?
}

async fn timeout_until<T>(
    deadline: Instant,
    future: impl Future<Output = T>,
) -> Result<T, DaemonError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(DaemonError::new(
            DomainErrorKind::DeadlineExceeded,
            "remote Session unary deadline elapsed",
        ));
    }
    tokio::time::timeout(remaining, future).await.map_err(|_| {
        DaemonError::new(
            DomainErrorKind::DeadlineExceeded,
            "remote Session unary exceeded its absolute deadline",
        )
    })
}

async fn write_local_attachment_error_best_effort<Writer>(
    writer: &mut Writer,
    request_id: u64,
    error: &DaemonError,
    deadline: Instant,
) where
    Writer: AsyncWrite + Unpin,
{
    let bytes = crate::service::ServiceReply::error(request_id, error).bytes;
    let write = async {
        writer.write_all(&bytes).await?;
        writer.flush().await?;
        writer.shutdown().await
    };
    let _ = timeout_until(deadline, write).await;
}

fn transport_unavailable(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::TransportUnavailable, detail)
}

fn outcome_unknown() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::OperationOutcomeUnknown,
        "remote Session mutation may have committed but no complete response was received",
    )
}

fn malformed(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::collections::BTreeSet;
    use std::collections::{BTreeMap, VecDeque};
    use std::fs;
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};
    #[cfg(unix)]
    use std::task::{Context, Poll};
    use std::time::Duration;

    #[cfg(unix)]
    use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
    use tokio::sync::{mpsc, oneshot};
    use zterm_core::{
        AttachmentPrincipal, AuthGeneration, AuthorizationStatus, DaemonIncarnation,
        DeviceDisplayName, OperationId, OperationLease, ResourceLimits, SessionId,
    };
    use zterm_platform::pty::{ExplicitPtyCommand, PtyHost, PtySize};
    use zterm_platform::user_state::UserPaths;
    use zterm_proto::{WireKind, encode_message};

    use super::*;
    use crate::authorization::AuthorizationRegistry;
    #[cfg(unix)]
    use crate::remote_attachment::{
        BoxAttachmentStream, OpenedAttachmentEpoch, RemoteAttachmentDemand,
        RemoteAttachmentTransport,
    };
    use crate::session::SessionService;
    use crate::session_wire::{SessionRequestContext, SessionWireLimits, SessionWireServer};
    use crate::store::{DeviceAuthorization, StateStore, StoreActor};

    #[derive(Clone)]
    struct FakeTransport {
        state: Arc<Mutex<FakeState>>,
    }

    struct FakeState {
        demand_targets: Vec<DeviceId>,
        requests: Vec<Vec<u8>>,
        outcomes: VecDeque<ScriptedOutcome>,
    }

    enum ScriptedOutcome {
        Response(DecodedFrame),
        ResponseBytes(Vec<u8>),
        PostWrite,
    }

    struct FakeDemand {
        state: Arc<Mutex<FakeState>>,
    }

    #[derive(Clone)]
    struct CommittedLossTransport {
        server: SessionWireServer,
        context: SessionRequestContext,
        state: Arc<Mutex<CommittedLossState>>,
        committed: mpsc::Sender<CommittedMutation>,
    }

    #[derive(Default)]
    struct CommittedLossState {
        demand_targets: Vec<DeviceId>,
        attempts_by_request: BTreeMap<u64, usize>,
        requests: Vec<(u64, Vec<u8>)>,
        responses: Vec<(u64, Vec<u8>)>,
    }

    struct CommittedLossDemand {
        transport: CommittedLossTransport,
    }

    struct CommittedMutation {
        request_id: u64,
        kind: WireKind,
        release_retry: oneshot::Sender<()>,
    }

    #[derive(Clone)]
    #[cfg(unix)]
    struct TaskPrivateSharedTransport {
        server: SessionWireServer,
        context: SessionRequestContext,
        state: Arc<Mutex<TaskPrivateTransportState>>,
    }

    #[derive(Default)]
    #[cfg(unix)]
    struct TaskPrivateTransportState {
        owned_targets: BTreeSet<DeviceId>,
        attachment_demands: usize,
        active_attachment_demands: usize,
        dropped_attachment_demands: usize,
        attachment_opens: usize,
        unary_demands: usize,
        dropped_unary_demands: usize,
        unary_exchanges: usize,
        streams: VecDeque<BoxAttachmentStream>,
    }

    #[cfg(unix)]
    struct SharedAttachmentDemand {
        state: Arc<Mutex<TaskPrivateTransportState>>,
    }

    #[cfg(unix)]
    struct SharedUnaryDemand {
        transport: TaskPrivateSharedTransport,
    }

    #[cfg(unix)]
    struct ObservedPendingWriteStream {
        inner: DuplexStream,
        first_pending: Option<oneshot::Sender<()>>,
    }

    #[cfg(unix)]
    struct AbortOnDrop<T> {
        task: Option<tokio::task::JoinHandle<T>>,
    }

    #[cfg(unix)]
    struct LocalBridgePeer {
        stream: DuplexStream,
        decoder: FrameDecoder,
        queued: VecDeque<DecodedFrame>,
        session_id: SessionId,
        attachment_id: AttachmentId,
        next_request_id: u64,
    }

    impl FakeTransport {
        fn scripted(outcomes: impl IntoIterator<Item = ScriptedOutcome>) -> Self {
            Self {
                state: Arc::new(Mutex::new(FakeState {
                    demand_targets: Vec::new(),
                    requests: Vec::new(),
                    outcomes: outcomes.into_iter().collect(),
                })),
            }
        }

        fn state(&self) -> MutexGuard<'_, FakeState> {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }

    #[cfg(unix)]
    impl TaskPrivateSharedTransport {
        fn new(
            server: SessionWireServer,
            context: SessionRequestContext,
            streams: impl IntoIterator<Item = BoxAttachmentStream>,
        ) -> Self {
            Self {
                server,
                context,
                state: Arc::new(Mutex::new(TaskPrivateTransportState {
                    streams: streams.into_iter().collect(),
                    ..TaskPrivateTransportState::default()
                })),
            }
        }

        fn state(&self) -> MutexGuard<'_, TaskPrivateTransportState> {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }

    #[cfg(unix)]
    impl RemoteAttachmentTransport for TaskPrivateSharedTransport {
        fn demand<'a>(
            &'a self,
            target: DeviceId,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<Box<dyn RemoteAttachmentDemand>, DaemonError>> {
            Box::pin(async move {
                let mut state = self.state();
                state.owned_targets.insert(target);
                state.attachment_demands += 1;
                state.active_attachment_demands += 1;
                drop(state);
                Ok(Box::new(SharedAttachmentDemand {
                    state: Arc::clone(&self.state),
                }) as Box<dyn RemoteAttachmentDemand>)
            })
        }
    }

    #[cfg(unix)]
    impl RemoteAttachmentDemand for SharedAttachmentDemand {
        fn open<'a>(
            &'a mut self,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<OpenedAttachmentEpoch, DaemonError>> {
            let stream = {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.attachment_opens += 1;
                state.streams.pop_front()
            };
            Box::pin(async move {
                stream
                    .map(OpenedAttachmentEpoch::unobserved)
                    .ok_or_else(|| {
                        DaemonError::new(
                            DomainErrorKind::ResourceExhausted,
                            "task-private transport fixture exhausted its bounded attachment streams",
                        )
                    })
            })
        }
    }

    #[cfg(unix)]
    impl Drop for SharedAttachmentDemand {
        fn drop(&mut self) {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.active_attachment_demands = state.active_attachment_demands.saturating_sub(1);
            state.dropped_attachment_demands += 1;
        }
    }

    #[cfg(unix)]
    impl RemoteUnaryTransport for TaskPrivateSharedTransport {
        fn demand<'a>(
            &'a self,
            target: DeviceId,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<Box<dyn RemoteUnaryDemand>, DaemonError>> {
            Box::pin(async move {
                let mut state = self.state();
                state.owned_targets.insert(target);
                state.unary_demands += 1;
                drop(state);
                Ok(Box::new(SharedUnaryDemand {
                    transport: self.clone(),
                }) as Box<dyn RemoteUnaryDemand>)
            })
        }
    }

    #[cfg(unix)]
    impl RemoteUnaryDemand for SharedUnaryDemand {
        fn exchange<'a>(
            &'a mut self,
            request: &'a [u8],
            deadline: Instant,
        ) -> BoxFuture<'a, Result<DecodedFrame, RemoteAttemptError>> {
            Box::pin(async move {
                self.transport.state().unary_exchanges += 1;
                let (response, _) = exchange_with_session_wire(
                    &self.transport.server,
                    &self.transport.context,
                    request,
                    deadline,
                )
                .await?;
                Ok(response)
            })
        }
    }

    #[cfg(unix)]
    impl Drop for SharedUnaryDemand {
        fn drop(&mut self) {
            self.transport.state().dropped_unary_demands += 1;
        }
    }

    #[cfg(unix)]
    impl AsyncRead for ObservedPendingWriteStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_read(context, buffer)
        }
    }

    #[cfg(unix)]
    impl AsyncWrite for ObservedPendingWriteStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            let result = Pin::new(&mut self.inner).poll_write(context, bytes);
            if result.is_pending()
                && let Some(first_pending) = self.first_pending.take()
            {
                let _ = first_pending.send(());
            }
            result
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_flush(context)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_shutdown(context)
        }
    }

    #[cfg(unix)]
    impl<T> AbortOnDrop<T> {
        fn new(task: tokio::task::JoinHandle<T>) -> Self {
            Self { task: Some(task) }
        }

        fn is_finished(&self) -> bool {
            self.task
                .as_ref()
                .is_none_or(tokio::task::JoinHandle::is_finished)
        }

        fn abort(&self) {
            if let Some(task) = self.task.as_ref() {
                task.abort();
            }
        }

        async fn join(mut self) -> Result<T, tokio::task::JoinError> {
            let result = self.task.as_mut().expect("guarded task exists").await;
            self.task.take();
            result
        }
    }

    #[cfg(unix)]
    impl<T> Drop for AbortOnDrop<T> {
        fn drop(&mut self) {
            if let Some(task) = self.task.take() {
                task.abort();
            }
        }
    }

    #[cfg(unix)]
    impl LocalBridgePeer {
        fn pair(session_id: SessionId, attachment_id: AttachmentId) -> (Self, DuplexStream) {
            let (stream, bridge) = duplex(64 * 1024);
            (
                Self {
                    stream,
                    decoder: FrameDecoder::new(),
                    queued: VecDeque::new(),
                    session_id,
                    attachment_id,
                    next_request_id: 2,
                },
                bridge,
            )
        }

        async fn next_before(&mut self, deadline: Instant) -> DecodedFrame {
            if let Some(frame) = self.queued.pop_front() {
                return frame;
            }
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
                let mut buffer = [0_u8; 4 * 1024];
                loop {
                    let read = self
                        .stream
                        .read(&mut buffer)
                        .await
                        .expect("read socket-free local bridge frame");
                    assert!(read > 0, "local bridge closed before its expected frame");
                    self.queued.extend(
                        self.decoder
                            .feed(&buffer[..read])
                            .expect("decode socket-free local bridge frame"),
                    );
                    if let Some(frame) = self.queued.pop_front() {
                        return frame;
                    }
                }
            })
            .await
            .expect("local bridge frame arrived before the absolute deadline")
        }

        async fn send_before<Message: prost::Message>(
            &mut self,
            kind: WireKind,
            message: &Message,
            deadline: Instant,
        ) {
            let request_id = self.next_request_id;
            self.next_request_id = self
                .next_request_id
                .checked_add(1)
                .expect("socket-free local bridge request IDs remain bounded");
            let bytes = encode_message(kind, request_id, 0, message)
                .expect("encode bounded socket-free local bridge frame");
            tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                self.stream.write_all(&bytes),
            )
            .await
            .expect("local bridge write completed before the absolute deadline")
            .expect("write socket-free local bridge frame");
        }

        async fn detach_before(&mut self, deadline: Instant) {
            self.send_before(
                WireKind::TerminalDetach,
                &v1::TerminalDetach {
                    attachment_id: Some(self.attachment_id.into()),
                },
                deadline,
            )
            .await;
            tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                self.stream.shutdown(),
            )
            .await
            .expect("local bridge shutdown completed before the absolute deadline")
            .expect("shutdown socket-free local bridge peer");
        }
    }

    impl RemoteUnaryTransport for FakeTransport {
        fn demand<'a>(
            &'a self,
            target: DeviceId,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<Box<dyn RemoteUnaryDemand>, DaemonError>> {
            Box::pin(async move {
                self.state().demand_targets.push(target);
                Ok(Box::new(FakeDemand {
                    state: Arc::clone(&self.state),
                }) as Box<dyn RemoteUnaryDemand>)
            })
        }
    }

    impl RemoteUnaryDemand for FakeDemand {
        fn exchange<'a>(
            &'a mut self,
            request: &'a [u8],
            deadline: Instant,
        ) -> BoxFuture<'a, Result<DecodedFrame, RemoteAttemptError>> {
            Box::pin(async move {
                let outcome = {
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.requests.push(request.to_vec());
                    state.outcomes.pop_front()
                };
                match outcome {
                    Some(ScriptedOutcome::Response(response)) => Ok(response),
                    Some(ScriptedOutcome::ResponseBytes(bytes)) => {
                        let capacity = bytes.len().max(1) + 1;
                        let (mut reader, mut writer) = duplex(capacity);
                        writer
                            .write_all(&bytes)
                            .await
                            .expect("write scripted remote response");
                        writer
                            .shutdown()
                            .await
                            .expect("finish scripted remote response");
                        read_exact_response(&mut reader, deadline).await
                    }
                    Some(ScriptedOutcome::PostWrite) => Err(RemoteAttemptError::PostWrite(
                        transport_unavailable("injected ambiguous response loss"),
                    )),
                    None => Err(RemoteAttemptError::PreWrite(DaemonError::new(
                        DomainErrorKind::ResourceExhausted,
                        "fake transport has no scripted attempt",
                    ))),
                }
            })
        }
    }

    impl CommittedLossTransport {
        fn new(
            server: SessionWireServer,
            context: SessionRequestContext,
        ) -> (Self, mpsc::Receiver<CommittedMutation>) {
            let (committed, receiver) = mpsc::channel(1);
            (
                Self {
                    server,
                    context,
                    state: Arc::new(Mutex::new(CommittedLossState::default())),
                    committed,
                },
                receiver,
            )
        }

        fn state(&self) -> MutexGuard<'_, CommittedLossState> {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }

    impl RemoteUnaryTransport for CommittedLossTransport {
        fn demand<'a>(
            &'a self,
            target: DeviceId,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<Box<dyn RemoteUnaryDemand>, DaemonError>> {
            Box::pin(async move {
                self.state().demand_targets.push(target);
                Ok(Box::new(CommittedLossDemand {
                    transport: self.clone(),
                }) as Box<dyn RemoteUnaryDemand>)
            })
        }
    }

    impl RemoteUnaryDemand for CommittedLossDemand {
        fn exchange<'a>(
            &'a mut self,
            request: &'a [u8],
            deadline: Instant,
        ) -> BoxFuture<'a, Result<DecodedFrame, RemoteAttemptError>> {
            Box::pin(async move {
                let request_frame =
                    decode_exact_frame(request).map_err(RemoteAttemptError::PreWrite)?;
                let attempt = {
                    let mut state = self.transport.state();
                    let attempt = state
                        .attempts_by_request
                        .entry(request_frame.request_id)
                        .or_default();
                    *attempt += 1;
                    let current = *attempt;
                    state
                        .requests
                        .push((request_frame.request_id, request.to_vec()));
                    current
                };

                let (response_frame, response) = exchange_with_session_wire(
                    &self.transport.server,
                    &self.transport.context,
                    request,
                    deadline,
                )
                .await?;
                self.transport
                    .state()
                    .responses
                    .push((request_frame.request_id, response));

                if attempt == 1 && response_frame.kind == WireKind::SessionMutateResponse {
                    let (release_retry, released) = oneshot::channel();
                    self.transport
                        .committed
                        .send(CommittedMutation {
                            request_id: request_frame.request_id,
                            kind: request_frame.kind,
                            release_retry,
                        })
                        .await
                        .map_err(|_| {
                            RemoteAttemptError::PostWrite(transport_unavailable(
                                "committed-response observer was dropped",
                            ))
                        })?;
                    released.await.map_err(|_| {
                        RemoteAttemptError::PostWrite(transport_unavailable(
                            "committed-response barrier was dropped",
                        ))
                    })?;
                    return Err(RemoteAttemptError::PostWrite(transport_unavailable(
                        "injected loss after the host encoded its committed response",
                    )));
                }
                Ok(response_frame)
            })
        }
    }

    async fn exchange_with_session_wire(
        server: &SessionWireServer,
        context: &SessionRequestContext,
        request: &[u8],
        deadline: Instant,
    ) -> Result<(DecodedFrame, Vec<u8>), RemoteAttemptError> {
        let (mut client_stream, server_stream) = duplex(64 * 1024);
        let server_exchange = server.handle_remote_stream(
            server_stream,
            context.clone(),
            SessionWireLimits::default(),
            deadline,
        );
        let client_exchange = async {
            client_stream.write_all(request).await.map_err(|_| {
                RemoteAttemptError::PostWrite(transport_unavailable("test request write failed"))
            })?;
            client_stream.shutdown().await.map_err(|_| {
                RemoteAttemptError::PostWrite(transport_unavailable("test request finish failed"))
            })?;
            let mut response = Vec::new();
            client_stream
                .read_to_end(&mut response)
                .await
                .map_err(|_| {
                    RemoteAttemptError::PostWrite(transport_unavailable(
                        "test response read failed",
                    ))
                })?;
            Ok::<_, RemoteAttemptError>(response)
        };
        let (server_result, response) =
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
                tokio::join!(server_exchange, client_exchange)
            })
            .await
            .map_err(|_| {
                RemoteAttemptError::PostWrite(DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    "test Session wire exchange exceeded its absolute deadline",
                ))
            })?;
        let response = response?;
        server_result.map_err(RemoteAttemptError::PostWrite)?;
        let response_frame =
            decode_exact_frame(&response).map_err(RemoteAttemptError::PostWrite)?;
        Ok((response_frame, response))
    }

    #[tokio::test]
    async fn local_attachment_error_flush_is_deadline_bounded() {
        let (_peer, mut stream) = duplex(1);
        let error = transport_unavailable("injected unavailable attachment transport");
        tokio::time::timeout(
            Duration::from_secs(1),
            write_local_attachment_error_best_effort(
                &mut stream,
                17,
                &error,
                Instant::now() + Duration::from_millis(10),
            ),
        )
        .await
        .expect("a stalled local error write releases at its absolute deadline");
    }

    #[test]
    fn canonical_self_target_is_rejected_without_an_outbound_directory_row() {
        let own_device_id = device(5);
        let temporary = tempfile::tempdir().expect("temporary resolver state root");
        let home = temporary.path().join("home");
        fs::create_dir(&home).expect("test home");
        let paths = UserPaths::for_test(
            nix::unistd::Uid::effective().as_raw(),
            home.clone(),
            home.join(".zterm"),
            temporary.path().join("run"),
        );
        paths
            .prepare_state_directories()
            .expect("state directories");
        let actor =
            StoreActor::start(StateStore::open(&paths).expect("state store")).expect("store actor");
        let service =
            RemoteSessionService::local_only(own_device_id, DeviceDirectory::new(actor.handle()));

        let error = service
            .resolve(
                &own_device_id.to_string(),
                Instant::now() + Duration::from_secs(1),
            )
            .expect_err("canonical self target always requires the reserved local selector");
        assert_eq!(error.kind(), DomainErrorKind::InvalidTargetSelector);
        actor.shutdown();
    }

    #[tokio::test]
    async fn strict_response_reader_requires_one_response_and_eof() {
        let response = encode_message(
            WireKind::SessionListResponse,
            41,
            0,
            &v1::SessionListResponse { sessions: vec![] },
        )
        .expect("bounded response");
        let (mut reader, mut writer) = duplex(4096);
        let write = tokio::spawn(async move {
            writer.write_all(&response).await.expect("write response");
            writer.shutdown().await.expect("finish response");
        });
        let frame = read_exact_response(&mut reader, Instant::now() + Duration::from_secs(1))
            .await
            .unwrap_or_else(|_| panic!("one response plus EOF must succeed"));
        write.await.expect("writer task");
        assert_eq!(frame.request_id, 41);
        assert_eq!(frame.kind, WireKind::SessionListResponse);

        let first = encode_message(
            WireKind::SessionListResponse,
            42,
            0,
            &v1::SessionListResponse { sessions: vec![] },
        )
        .expect("first response");
        let second = encode_message(
            WireKind::SessionListResponse,
            42,
            0,
            &v1::SessionListResponse { sessions: vec![] },
        )
        .expect("second response");
        let (mut reader, mut writer) = duplex(4096);
        let write = tokio::spawn(async move {
            writer.write_all(&first).await.expect("write first");
            writer.write_all(&second).await.expect("write second");
            writer.shutdown().await.expect("finish responses");
        });
        let error = read_exact_response(&mut reader, Instant::now() + Duration::from_secs(1))
            .await
            .expect_err("multiple responses must fail");
        write.await.expect("writer task");
        assert!(matches!(
            error,
            RemoteAttemptError::PostWrite(error)
                if error.kind() == DomainErrorKind::MalformedFrame
        ));
    }

    #[tokio::test]
    async fn request_boundary_rejects_multiple_frames_before_connection_demand() {
        let target = device(6);
        let mut request = list_request(target, 31);
        request.extend_from_slice(&list_request(target, 32));
        let transport = FakeTransport::scripted([]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let error = client
            .execute_preencoded(
                target,
                31,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect_err("one outbound stream carries exactly one request");
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
        assert!(transport.state().demand_targets.is_empty());
    }

    #[test]
    fn request_boundary_rejects_nested_terminal_and_trailing_inner_frames() {
        let target = device(15);
        let nested = encode_message(
            WireKind::LocalSessionUnaryRequest,
            131,
            1_000,
            &v1::LocalSessionUnaryRequest {
                target_device_id: Some(target.into()),
                frame: list_request(target, 131),
            },
        )
        .expect("bounded nested fixture");
        let terminal = encode_message(
            WireKind::TerminalInput,
            132,
            1_000,
            &v1::TerminalInput {
                operation_id: None,
                attachment_id: None,
                bytes: Vec::new(),
            },
        )
        .expect("bounded terminal fixture");
        let mut trailing = list_request(target, 133);
        trailing.extend_from_slice(&[5, 1]);

        for request in [nested, terminal, trailing] {
            assert_eq!(
                RequestContract::inspect(&request, target)
                    .expect_err("the envelope accepts exactly one unary Session control frame")
                    .kind(),
                DomainErrorKind::MalformedFrame
            );
        }
    }

    #[tokio::test]
    async fn forwarding_envelope_request_id_must_match_before_connection_demand() {
        let target = device(16);
        let request = list_request(target, 141);
        let transport = FakeTransport::scripted([]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let error = client
            .execute_preencoded(
                target,
                142,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect_err("outer and inner request IDs must match before forwarding");
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
        assert!(transport.state().demand_targets.is_empty());
    }

    #[tokio::test]
    async fn read_only_ambiguity_retries_once_with_one_demand_and_exact_bytes() {
        let target = device(7);
        let request = list_request(target, 51);
        let response = decoded_message(
            WireKind::SessionListResponse,
            51,
            &v1::SessionListResponse { sessions: vec![] },
        );
        let transport = FakeTransport::scripted([
            ScriptedOutcome::PostWrite,
            ScriptedOutcome::Response(response),
        ]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let reply = client
            .execute_preencoded(
                target,
                51,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect("safe retry succeeds");
        assert_eq!(reply.request_id, 51);
        assert_eq!(reply.kind, WireKind::SessionListResponse);
        let state = transport.state();
        assert_eq!(state.demand_targets, [target]);
        assert_eq!(state.requests, [request.clone(), request]);
    }

    #[tokio::test]
    async fn operation_lease_post_write_failure_is_stateful_and_never_retried() {
        let target = device(18);
        let request = lease_request(target, 161);
        let unused_second_response = decoded_message(
            WireKind::SessionOperationLeaseResponse,
            161,
            &v1::SessionOperationLeaseResponse {
                lease: Some(v1::OperationLease {
                    daemon_incarnation: vec![4; DaemonIncarnation::LENGTH],
                    ordinal: 29,
                }),
            },
        );
        let transport = FakeTransport::scripted([
            ScriptedOutcome::PostWrite,
            ScriptedOutcome::Response(unused_second_response),
        ]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let error = client
            .execute_preencoded(
                target,
                161,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect_err("ambiguous lease allocation must not open another service stream");
        assert_eq!(error.kind(), DomainErrorKind::TransportUnavailable);
        let state = transport.state();
        assert_eq!(state.demand_targets, [target]);
        assert_eq!(state.requests, [request]);
        assert_eq!(state.outcomes.len(), 1);
    }

    #[tokio::test]
    async fn mutation_retry_preserves_host_lease_operation_and_frame_bytes() {
        let target = device(8);
        let request = create_request(target, 61);
        let response = decoded_message(
            WireKind::SessionMutateResponse,
            61,
            &v1::SessionMutateResponse {
                session: Some(valid_session_summary(8)),
            },
        );
        let transport = FakeTransport::scripted([
            ScriptedOutcome::PostWrite,
            ScriptedOutcome::Response(response),
        ]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        client
            .execute_preencoded(
                target,
                61,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect("same operation retry succeeds");
        let state = transport.state();
        assert_eq!(state.demand_targets, [target]);
        assert_eq!(state.requests.len(), 2);
        assert_eq!(state.requests[0], state.requests[1]);
        assert_eq!(state.requests[0], request);

        let frame = decode_exact_frame(&state.requests[1]).expect("recorded request decodes");
        let message: v1::SessionCreateRequest = frame
            .decode_message(WireKind::SessionCreateRequest)
            .expect("create request");
        let operation: OperationId = message
            .operation_id
            .expect("operation ID")
            .try_into()
            .expect("valid operation ID");
        assert_eq!(operation.lease.ordinal, 23);
        assert_eq!(operation.sequence, 5);
        assert_eq!(
            operation.lease.daemon_incarnation,
            DaemonIncarnation::from_array([9; 16])
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn committed_create_rename_and_close_replay_through_the_real_wire_owner() {
        let host = device(0x81);
        let remote = device(0x82);
        let accepted_generation = AuthGeneration::new(7).expect("non-zero generation");
        let authorization = authorized_registry(remote, accepted_generation);
        let temporary = tempfile::tempdir().expect("temporary committed-replay fixture");
        let spawn_count = Arc::new(AtomicUsize::new(0));
        let sessions = counted_session_service(
            host,
            temporary.path().to_path_buf(),
            Arc::clone(&spawn_count),
        );
        let context = SessionRequestContext::RemoteAuthenticated {
            own_device_id: host,
            remote_device_id: remote,
            accepted_generation,
            authorization,
            commit_first_poll_observer: None,
        };
        let (transport, mut committed) =
            CommittedLossTransport::new(SessionWireServer::new(sessions.clone()), context);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let lease_request_id = 200;
        let lease_request = lease_request(host, lease_request_id);
        let lease_response = client
            .execute_preencoded(
                host,
                lease_request_id,
                &lease_request,
                Instant::now() + Duration::from_secs(5),
            )
            .await
            .expect("the host issues one replay lease through the real wire server");
        let lease: OperationLease = lease_response
            .decode_message::<v1::SessionOperationLeaseResponse>(
                WireKind::SessionOperationLeaseResponse,
            )
            .expect("decode daemon-issued lease response")
            .lease
            .expect("lease response contains its lease")
            .try_into()
            .expect("daemon-issued lease is valid");
        assert_eq!(transport.state().demand_targets, [host]);
        assert_single_attempt(&transport, lease_request_id, &lease_request);

        let create_operation = OperationId { lease, sequence: 1 };
        let create_request_id = 201;
        let create_request =
            create_mutation_request(host, create_request_id, create_operation, "wire-replay");
        let create_client = client.clone();
        let create_bytes = create_request.clone();
        let create_task = tokio::spawn(async move {
            create_client
                .execute_preencoded(
                    host,
                    create_request_id,
                    &create_bytes,
                    Instant::now() + Duration::from_secs(5),
                )
                .await
        });
        let create_commit = committed
            .recv()
            .await
            .expect("create commit reaches the encoded-response barrier");
        assert_eq!(
            (create_commit.request_id, create_commit.kind),
            (create_request_id, WireKind::SessionCreateRequest)
        );
        let created = sessions.list().expect("list committed create");
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].name.as_str(), "wire-replay");
        assert_eq!(spawn_count.load(Ordering::Acquire), 1);
        create_commit
            .release_retry
            .send(())
            .expect("release create retry after inspecting committed state");
        let create_response = create_task
            .await
            .expect("create client task")
            .expect("create retry replays its result");
        let created_response = mutation_summary(&create_response);
        let session_id = created_response.session_id;
        assert_eq!(created_response.name.as_str(), "wire-replay");
        assert_exact_mutation_replay(
            &transport,
            create_request_id,
            &create_request,
            create_operation,
        );
        assert_eq!(sessions.list().expect("list replayed create").len(), 1);
        assert_eq!(spawn_count.load(Ordering::Acquire), 1);

        let mismatch_request_id = 202;
        let mismatch_request = create_mutation_request(
            host,
            mismatch_request_id,
            create_operation,
            "must-not-spawn",
        );
        let mismatch = client
            .execute_preencoded(
                host,
                mismatch_request_id,
                &mismatch_request,
                Instant::now() + Duration::from_secs(5),
            )
            .await
            .expect("a complete correlated typed mismatch is terminal");
        assert_eq!(mismatch.kind, WireKind::ServiceErrorResponse);
        assert_eq!(
            decode_session_service_error(&mismatch)
                .expect("decode fingerprint mismatch")
                .kind(),
            DomainErrorKind::OperationOutcomeUnknown
        );
        assert_single_attempt(&transport, mismatch_request_id, &mismatch_request);
        assert_eq!(sessions.list().expect("list after mismatch").len(), 1);
        assert_eq!(spawn_count.load(Ordering::Acquire), 1);

        let rename_operation = OperationId { lease, sequence: 2 };
        let rename_request_id = 203;
        let rename_request = rename_mutation_request(
            host,
            rename_request_id,
            rename_operation,
            session_id,
            "renamed-once",
        );
        let rename_client = client.clone();
        let rename_bytes = rename_request.clone();
        let rename_task = tokio::spawn(async move {
            rename_client
                .execute_preencoded(
                    host,
                    rename_request_id,
                    &rename_bytes,
                    Instant::now() + Duration::from_secs(5),
                )
                .await
        });
        let rename_commit = committed
            .recv()
            .await
            .expect("rename commit reaches the encoded-response barrier");
        assert_eq!(
            (rename_commit.request_id, rename_commit.kind),
            (rename_request_id, WireKind::SessionRenameRequest)
        );
        assert_eq!(
            sessions.list().expect("list committed rename")[0]
                .name
                .as_str(),
            "renamed-once"
        );

        // Move the live name again only after the first response is complete.
        // A replay must return the cached first result without reapplying the
        // earlier rename over this later independent transition.
        let principal = AttachmentPrincipal::RemoteEndpoint {
            device_id: remote,
            auth_generation: accepted_generation.get(),
        };
        sessions
            .rename(
                principal,
                OperationId { lease, sequence: 3 },
                session_id,
                SessionName::new("after-replay-barrier").expect("intervening name"),
            )
            .expect("independent rename after committed-response barrier");
        rename_commit
            .release_retry
            .send(())
            .expect("release rename retry after the intervening transition");
        let rename_response = rename_task
            .await
            .expect("rename client task")
            .expect("rename retry replays its cached result");
        assert_eq!(
            mutation_summary(&rename_response).name.as_str(),
            "renamed-once"
        );
        assert_eq!(
            sessions.list().expect("list after replayed rename")[0]
                .name
                .as_str(),
            "after-replay-barrier"
        );
        assert_exact_mutation_replay(
            &transport,
            rename_request_id,
            &rename_request,
            rename_operation,
        );
        assert_eq!(spawn_count.load(Ordering::Acquire), 1);

        let close_operation = OperationId { lease, sequence: 4 };
        let close_request_id = 204;
        let close_request =
            close_mutation_request(host, close_request_id, close_operation, session_id);
        let close_client = client.clone();
        let close_bytes = close_request.clone();
        let close_task = tokio::spawn(async move {
            close_client
                .execute_preencoded(
                    host,
                    close_request_id,
                    &close_bytes,
                    Instant::now() + Duration::from_secs(5),
                )
                .await
        });
        let close_commit = committed
            .recv()
            .await
            .expect("close commit reaches the encoded-response barrier");
        assert_eq!(
            (close_commit.request_id, close_commit.kind),
            (close_request_id, WireKind::SessionCloseRequest)
        );
        assert!(
            sessions
                .list()
                .expect("list after committed close")
                .is_empty()
        );
        close_commit
            .release_retry
            .send(())
            .expect("release close retry after inspecting committed state");
        let close_response = close_task
            .await
            .expect("close client task")
            .expect("close retry replays its cached success");
        assert_eq!(
            mutation_summary(&close_response).name.as_str(),
            "after-replay-barrier"
        );
        assert_exact_mutation_replay(
            &transport,
            close_request_id,
            &close_request,
            close_operation,
        );
        assert!(sessions.list().expect("final Session list").is_empty());
        assert_eq!(spawn_count.load(Ordering::Acquire), 1);

        let state = transport.state();
        assert_eq!(
            state.demand_targets,
            [host, host, host, host, host],
            "lease, create, mismatch, rename, and close each own one demand"
        );
        assert!(committed.try_recv().is_err());
    }

    #[tokio::test]
    async fn malformed_or_truncated_mutation_replies_retry_then_become_outcome_unknown() {
        let target = device(12);
        let request = create_request(target, 101);
        let mut truncated = encode_message(
            WireKind::SessionMutateResponse,
            101,
            0,
            &v1::SessionMutateResponse {
                session: Some(valid_session_summary(12)),
            },
        )
        .expect("bounded mutation response");
        truncated.pop().expect("truncate encoded response");
        let malformed = encode_message(
            WireKind::SessionMutateResponse,
            101,
            0,
            &v1::SessionMutateResponse { session: None },
        )
        .expect("well-framed but incomplete typed mutation response");
        let transport = FakeTransport::scripted([
            ScriptedOutcome::ResponseBytes(truncated),
            ScriptedOutcome::ResponseBytes(malformed),
        ]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let error = client
            .execute_preencoded(
                target,
                101,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect_err("two unresolved post-write mutation replies are outcome unknown");
        assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
        let state = transport.state();
        assert_eq!(state.demand_targets, [target]);
        assert_eq!(state.requests, [request.clone(), request]);
    }

    #[tokio::test]
    async fn wrong_kind_or_request_id_mutation_replies_are_post_write_ambiguity() {
        let target = device(13);
        let request = create_request(target, 111);
        let wrong_kind = decoded_message(
            WireKind::SessionListResponse,
            111,
            &v1::SessionListResponse { sessions: vec![] },
        );
        let wrong_request_id = decoded_message(
            WireKind::SessionMutateResponse,
            112,
            &v1::SessionMutateResponse {
                session: Some(valid_session_summary(13)),
            },
        );
        let transport = FakeTransport::scripted([
            ScriptedOutcome::Response(wrong_kind),
            ScriptedOutcome::Response(wrong_request_id),
        ]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let error = client
            .execute_preencoded(
                target,
                111,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect_err("uncorrelated mutation replies cannot prove the operation outcome");
        assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
        let state = transport.state();
        assert_eq!(state.demand_targets, [target]);
        assert_eq!(state.requests, [request.clone(), request]);
    }

    #[tokio::test]
    async fn read_only_wrong_response_retries_once_and_preserves_protocol_distinctions() {
        let target = device(14);
        let request = list_request(target, 121);
        let wrong = decoded_message(
            WireKind::SessionMutateResponse,
            121,
            &v1::SessionMutateResponse { session: None },
        );
        let correct = decoded_message(
            WireKind::SessionListResponse,
            121,
            &v1::SessionListResponse { sessions: vec![] },
        );
        let transport = FakeTransport::scripted([
            ScriptedOutcome::Response(wrong),
            ScriptedOutcome::Response(correct),
        ]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let response = client
            .execute_preencoded(
                target,
                121,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect("safe read-only request retries an unresolved response");
        assert_eq!(response.kind, WireKind::SessionListResponse);
        assert_eq!(transport.state().requests, [request.clone(), request]);
    }

    #[tokio::test]
    async fn complete_typed_error_is_terminal_and_never_retried() {
        const UNTRUSTED_MESSAGE: &str = "REMOTE_SERVICE_ERROR_SENTINEL_4da9";
        let target = device(9);
        let request = create_request(target, 71);
        let response = decoded_message(
            WireKind::ServiceErrorResponse,
            71,
            &v1::ServiceError {
                code: DomainErrorKind::OperationOutcomeUnknown.code().to_owned(),
                message: UNTRUSTED_MESSAGE.to_owned(),
            },
        );
        let transport = FakeTransport::scripted([
            ScriptedOutcome::Response(response),
            ScriptedOutcome::PostWrite,
        ]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let reply = client
            .execute_preencoded(
                target,
                71,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect("complete typed error is returned to the local client");
        assert_eq!(reply.kind, WireKind::ServiceErrorResponse);
        let projected: v1::ServiceError = reply
            .decode_message(WireKind::ServiceErrorResponse)
            .expect("content-free typed error");
        assert_eq!(
            projected.code,
            DomainErrorKind::OperationOutcomeUnknown.code()
        );
        assert_eq!(projected.message, REMOTE_SESSION_FAILURE_DETAIL);
        assert!(!projected.message.contains(UNTRUSTED_MESSAGE));
        assert_eq!(transport.state().requests.len(), 1);
    }

    #[tokio::test]
    async fn fully_valid_expected_response_is_terminal_and_never_retried() {
        let target = device(17);
        let request = create_request(target, 151);
        let response = decoded_message(
            WireKind::SessionMutateResponse,
            151,
            &v1::SessionMutateResponse {
                session: Some(valid_session_summary(17)),
            },
        );
        let transport = FakeTransport::scripted([
            ScriptedOutcome::Response(response),
            ScriptedOutcome::PostWrite,
        ]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let reply = client
            .execute_preencoded(
                target,
                151,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect("a fully validated expected response is definitive");
        assert_eq!(reply.kind, WireKind::SessionMutateResponse);
        assert_eq!(transport.state().requests.len(), 1);
    }

    #[tokio::test]
    async fn final_mutation_ambiguity_is_outcome_unknown_without_fresh_execution() {
        let target = device(10);
        let request = create_request(target, 81);
        let transport =
            FakeTransport::scripted([ScriptedOutcome::PostWrite, ScriptedOutcome::PostWrite]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let error = client
            .execute_preencoded(
                target,
                81,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect_err("two ambiguous mutation attempts are terminal");
        assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
        let state = transport.state();
        assert_eq!(state.demand_targets, [target]);
        assert_eq!(state.requests, [request.clone(), request]);
    }

    #[tokio::test]
    async fn final_read_only_ambiguity_remains_a_transport_failure() {
        let target = device(11);
        let request = list_request(target, 91);
        let transport =
            FakeTransport::scripted([ScriptedOutcome::PostWrite, ScriptedOutcome::PostWrite]);
        let client = RemoteUnaryClient::new(Arc::new(transport.clone()));

        let error = client
            .execute_preencoded(
                target,
                91,
                &request,
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect_err("read-only ambiguity is not projected as mutation outcome unknown");
        assert_eq!(error.kind(), DomainErrorKind::TransportUnavailable);
        assert_eq!(transport.state().requests, [request.clone(), request]);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn task_private_shared_transport_keeps_attachment_and_unary_streams_independent() {
        let host = device(0xa1);
        let remote = device(0xa2);
        let accepted_generation = AuthGeneration::new(9).expect("non-zero generation");
        let authorization = authorized_registry(remote, accepted_generation);
        let temporary = tempfile::tempdir().expect("temporary stream-isolation fixture");
        let spawn_count = Arc::new(AtomicUsize::new(0));
        let sessions = counted_session_service(
            host,
            temporary.path().to_path_buf(),
            Arc::clone(&spawn_count),
        );
        let creator = sessions.local_principal(AttachmentId::from_array([0xa3; 16]));
        let creator_lease = sessions
            .issue_operation_lease(creator)
            .expect("stream-isolation creator lease");
        let summary = sessions
            .create(
                creator,
                OperationId {
                    lease: creator_lease,
                    sequence: 1,
                },
                SessionName::new("stream-isolation").expect("fixture session name"),
                None,
                None,
            )
            .expect("stream-isolation session creates");
        assert_eq!(spawn_count.load(Ordering::Acquire), 1);

        let server = SessionWireServer::new(sessions.clone());
        let context = SessionRequestContext::RemoteAuthenticated {
            own_device_id: host,
            remote_device_id: remote,
            accepted_generation,
            authorization,
            commit_first_poll_observer: None,
        };
        let deadline = Instant::now() + Duration::from_secs(5);

        // This fixture models only one task-private owner and independent
        // bounded streams. Production ConnectionBroker ownership/admission is
        // covered by its own socket-free and Linux real-Iroh gates.
        let (blocked_bridge, blocked_peer) = duplex(1);
        let (first_pending, first_pending_receiver) = oneshot::channel();
        let blocked_bridge = ObservedPendingWriteStream {
            inner: blocked_bridge,
            first_pending: Some(first_pending),
        };
        let (healthy_bridge, healthy_server_stream) = duplex(64 * 1024);
        let transport = Arc::new(TaskPrivateSharedTransport::new(
            server.clone(),
            context.clone(),
            [
                Box::new(blocked_bridge) as BoxAttachmentStream,
                Box::new(healthy_bridge) as BoxAttachmentStream,
            ],
        ));
        let attachment_client = RemoteAttachmentClient::with_test_transport(transport.clone());

        let first_local_id = AttachmentId::from_array([0xa4; 16]);
        let (first_local, first_local_stream) =
            LocalBridgePeer::pair(summary.session_id, first_local_id);
        let first_frame = existing_attachment_first_frame(host, summary.session_id, 1);
        let first_client = attachment_client.clone();
        let first_bridge_task = AbortOnDrop::new(tokio::spawn(async move {
            tokio::task::unconstrained(first_client.serve(
                host,
                first_local_id,
                first_local_stream,
                first_frame,
                SessionWireLimits::default(),
                deadline,
            ))
            .await
        }));
        tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            first_pending_receiver,
        )
        .await
        .expect("first attachment write reached its deterministic barrier")
        .expect("first attachment write observer remained live");
        assert!(
            !first_bridge_task.is_finished(),
            "a proven-pending attachment write must still own only its task and stream",
        );

        let healthy_server = server.clone();
        let healthy_context = context.clone();
        let healthy_server_task = AbortOnDrop::new(tokio::spawn(async move {
            healthy_server
                .handle_remote_stream(
                    healthy_server_stream,
                    healthy_context,
                    SessionWireLimits::default(),
                    deadline,
                )
                .await
        }));
        let second_local_id = AttachmentId::from_array([0xa5; 16]);
        let (mut second_local, second_local_stream) =
            LocalBridgePeer::pair(summary.session_id, second_local_id);
        let second_frame = existing_attachment_first_frame(host, summary.session_id, 2);
        let second_client = attachment_client;
        let second_bridge_task = AbortOnDrop::new(tokio::spawn(async move {
            second_client
                .serve(
                    host,
                    second_local_id,
                    second_local_stream,
                    second_frame,
                    SessionWireLimits::default(),
                    deadline,
                )
                .await
        }));

        let unary_client = RemoteUnaryClient::new(transport.clone());
        let unary_request = list_request(host, 3);
        let unary_task = AbortOnDrop::new(tokio::spawn(async move {
            unary_client
                .execute_preencoded(host, 3, &unary_request, deadline)
                .await
        }));

        let _second_revision = activate_bridge_before(&mut second_local, deadline).await;
        let unary_response =
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), unary_task.join())
                .await
                .expect("unary RPC progressed while the first attachment write remained pending")
                .expect("unary task joins")
                .expect("unary RPC succeeds through the real Session wire server");
        assert_eq!(unary_response.kind, WireKind::SessionListResponse);
        let listed: v1::SessionListResponse = unary_response
            .decode_message(WireKind::SessionListResponse)
            .expect("decode isolated unary response");
        assert_eq!(listed.sessions.len(), 1);
        let listed_id: SessionId = listed.sessions[0]
            .session_id
            .clone()
            .expect("listed session has an ID")
            .try_into()
            .expect("listed session ID is valid");
        assert_eq!(listed_id, summary.session_id);
        assert!(
            !first_bridge_task.is_finished(),
            "the stalled stream must remain isolated after peer streams complete work",
        );
        assert!(
            !second_bridge_task.is_finished(),
            "the healthy attachment remains active until its local view detaches",
        );

        {
            let state = transport.state();
            assert_eq!(state.owned_targets, BTreeSet::from([host]));
            assert_eq!(state.attachment_demands, 2);
            assert_eq!(state.active_attachment_demands, 2);
            assert_eq!(state.dropped_attachment_demands, 0);
            assert_eq!(state.attachment_opens, 2);
            assert_eq!(state.unary_demands, 1);
            assert_eq!(state.dropped_unary_demands, 1);
            assert_eq!(state.unary_exchanges, 1);
            assert!(state.streams.is_empty());
        }

        second_local.detach_before(deadline).await;

        tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            second_bridge_task.join(),
        )
        .await
        .expect("healthy attachment bridge releases before the absolute deadline")
        .expect("healthy attachment bridge joins")
        .expect("healthy attachment bridge exits cleanly");
        tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            healthy_server_task.join(),
        )
        .await
        .expect("healthy Session wire task releases before the absolute deadline")
        .expect("healthy Session wire task joins")
        .expect("healthy Session wire task exits cleanly");
        first_bridge_task.abort();
        let first_join = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            first_bridge_task.join(),
        )
        .await
        .expect("stalled attachment bridge releases before the absolute deadline")
        .expect_err("stalled attachment bridge is explicitly cancelled after isolation proof");
        assert!(first_join.is_cancelled());
        drop(first_local);
        drop(blocked_peer);

        {
            let state = transport.state();
            assert_eq!(state.active_attachment_demands, 0);
            assert_eq!(state.dropped_attachment_demands, 2);
            assert_eq!(state.dropped_unary_demands, 1);
            assert!(state.streams.is_empty());
        }
        assert_eq!(
            Arc::strong_count(&transport),
            1,
            "all client, demand, and task ownership returns to the fixture owner",
        );
        let surviving = sessions.list().expect("Session survives stream teardown");
        assert_eq!(surviving.len(), 1);
        assert_eq!(surviving[0].session_id, summary.session_id);
        sessions
            .shutdown()
            .expect("stream-isolation fixture shuts down");
    }

    #[cfg(unix)]
    async fn activate_bridge_before(client: &mut LocalBridgePeer, deadline: Instant) -> Revision {
        let mut saw_preparing = false;
        let mut saw_synchronizing = false;
        let snapshot_revision = loop {
            let frame = client.next_before(deadline).await;
            match frame.kind {
                WireKind::TerminalTransportStateEvent => {
                    let state: v1::TerminalTransportStateEvent = frame
                        .decode_message(WireKind::TerminalTransportStateEvent)
                        .expect("decode local bridge transport state");
                    let attachment_id: AttachmentId = state
                        .attachment_id
                        .expect("local bridge transport state has an attachment ID")
                        .try_into()
                        .expect("local bridge transport attachment ID is valid");
                    assert_eq!(attachment_id, client.attachment_id);
                    let state = v1::TerminalTransportState::try_from(state.state)
                        .expect("known local transport state");
                    match state {
                        v1::TerminalTransportState::Preparing => saw_preparing = true,
                        v1::TerminalTransportState::Synchronizing => {
                            assert!(saw_preparing);
                            saw_synchronizing = true;
                        }
                        _ => panic!("initial bridge state advanced out of order"),
                    }
                }
                WireKind::TerminalSnapshot => {
                    let snapshot: v1::TerminalSnapshot = frame
                        .decode_message(WireKind::TerminalSnapshot)
                        .expect("decode authoritative local bridge snapshot");
                    assert!(saw_preparing && saw_synchronizing);
                    let session_id: SessionId = snapshot
                        .session_id
                        .expect("local bridge snapshot has a Session ID")
                        .try_into()
                        .expect("local bridge snapshot Session ID is valid");
                    let attachment_id: AttachmentId = snapshot
                        .attachment_id
                        .expect("local bridge snapshot has an attachment ID")
                        .try_into()
                        .expect("local bridge snapshot attachment ID is valid");
                    assert_eq!(session_id, client.session_id);
                    assert_eq!(attachment_id, client.attachment_id);
                    let revision = Revision::new(snapshot.revision);
                    let local_attachment_id = client.attachment_id;
                    client
                        .send_before(
                            WireKind::TerminalSnapshotApplied,
                            &v1::TerminalSnapshotApplied {
                                attachment_id: Some(local_attachment_id.into()),
                                revision: revision.get(),
                            },
                            deadline,
                        )
                        .await;
                    break revision;
                }
                _ => panic!("healthy bridge omitted its authoritative initial snapshot"),
            }
        };

        let frame = client.next_before(deadline).await;
        assert_eq!(frame.kind, WireKind::TerminalTransportStateEvent);
        let state: v1::TerminalTransportStateEvent = frame
            .decode_message(WireKind::TerminalTransportStateEvent)
            .expect("decode local bridge activation state");
        let attachment_id: AttachmentId = state
            .attachment_id
            .expect("local bridge activation state has an attachment ID")
            .try_into()
            .expect("local bridge activation attachment ID is valid");
        assert_eq!(attachment_id, client.attachment_id);
        assert_eq!(
            v1::TerminalTransportState::try_from(state.state),
            Ok(v1::TerminalTransportState::Active)
        );
        snapshot_revision
    }

    #[cfg(unix)]
    fn existing_attachment_first_frame(
        target: DeviceId,
        session_id: SessionId,
        request_id: u64,
    ) -> FirstFrame {
        let bytes = encode_message(
            WireKind::TerminalAttachRequest,
            request_id,
            5_000,
            &v1::TerminalAttachRequest {
                target: Some(wire_target(target)),
                session_id: Some(session_id.into()),
                takeover: false,
                session_name: String::new(),
                create_main: false,
                viewport: Some(v1::TerminalViewport {
                    rows: 24,
                    columns: 80,
                }),
                resume_view_id: None,
                known_revision: None,
            },
        )
        .expect("encode bounded attachment first frame");
        let mut decoder = FrameDecoder::new();
        let mut queued = VecDeque::from(
            decoder
                .feed(&bytes)
                .expect("decode bounded attachment first frame"),
        );
        let frame = queued.pop_front().expect("one attachment first frame");
        assert!(queued.is_empty());
        FirstFrame {
            frame,
            decoder,
            queued,
        }
    }

    fn device(byte: u8) -> DeviceId {
        DeviceId::from_array([byte; DeviceId::LENGTH])
    }

    fn wire_target(target: DeviceId) -> v1::TargetSelector {
        v1::TargetSelector {
            target: Some(v1::target_selector::Target::Device(target.into())),
        }
    }

    fn list_request(target: DeviceId, request_id: u64) -> Vec<u8> {
        encode_message(
            WireKind::SessionListRequest,
            request_id,
            1_000,
            &v1::SessionListRequest {
                target: Some(wire_target(target)),
            },
        )
        .expect("bounded list request")
    }

    fn lease_request(target: DeviceId, request_id: u64) -> Vec<u8> {
        encode_message(
            WireKind::SessionOperationLeaseRequest,
            request_id,
            1_000,
            &v1::SessionOperationLeaseRequest {
                target: Some(wire_target(target)),
            },
        )
        .expect("bounded operation lease request")
    }

    fn create_request(target: DeviceId, request_id: u64) -> Vec<u8> {
        encode_message(
            WireKind::SessionCreateRequest,
            request_id,
            1_000,
            &v1::SessionCreateRequest {
                operation_id: Some(
                    OperationId {
                        lease: OperationLease {
                            daemon_incarnation: DaemonIncarnation::from_array([9; 16]),
                            ordinal: 23,
                        },
                        sequence: 5,
                    }
                    .into(),
                ),
                target: Some(wire_target(target)),
                name: "build".to_owned(),
                working_directory: String::new(),
                viewport: None,
            },
        )
        .expect("bounded create request")
    }

    fn create_mutation_request(
        target: DeviceId,
        request_id: u64,
        operation_id: OperationId,
        name: &str,
    ) -> Vec<u8> {
        encode_message(
            WireKind::SessionCreateRequest,
            request_id,
            5_000,
            &v1::SessionCreateRequest {
                operation_id: Some(operation_id.into()),
                target: Some(wire_target(target)),
                name: name.to_owned(),
                working_directory: String::new(),
                viewport: Some(zterm_core::terminal::TerminalSize::new(24, 80).into()),
            },
        )
        .expect("bounded committed create request")
    }

    fn rename_mutation_request(
        target: DeviceId,
        request_id: u64,
        operation_id: OperationId,
        session_id: SessionId,
        name: &str,
    ) -> Vec<u8> {
        encode_message(
            WireKind::SessionRenameRequest,
            request_id,
            5_000,
            &v1::SessionRenameRequest {
                operation_id: Some(operation_id.into()),
                target: Some(wire_target(target)),
                session_id: Some(session_id.into()),
                name: name.to_owned(),
            },
        )
        .expect("bounded committed rename request")
    }

    fn close_mutation_request(
        target: DeviceId,
        request_id: u64,
        operation_id: OperationId,
        session_id: SessionId,
    ) -> Vec<u8> {
        encode_message(
            WireKind::SessionCloseRequest,
            request_id,
            5_000,
            &v1::SessionCloseRequest {
                operation_id: Some(operation_id.into()),
                target: Some(wire_target(target)),
                session_id: Some(session_id.into()),
            },
        )
        .expect("bounded committed close request")
    }

    fn mutation_summary(frame: &DecodedFrame) -> SessionSummary {
        let response: v1::SessionMutateResponse = frame
            .decode_message(WireKind::SessionMutateResponse)
            .expect("decode mutation response");
        session_summary_from_wire(
            response
                .session
                .expect("mutation response contains summary"),
        )
        .expect("valid mutation summary")
    }

    fn operation_id_from_mutation(request: &[u8]) -> OperationId {
        let frame = decode_exact_frame(request).expect("recorded mutation frame");
        let operation_id = match frame.kind {
            WireKind::SessionCreateRequest => {
                frame
                    .decode_message::<v1::SessionCreateRequest>(WireKind::SessionCreateRequest)
                    .expect("decode recorded create")
                    .operation_id
            }
            WireKind::SessionRenameRequest => {
                frame
                    .decode_message::<v1::SessionRenameRequest>(WireKind::SessionRenameRequest)
                    .expect("decode recorded rename")
                    .operation_id
            }
            WireKind::SessionCloseRequest => {
                frame
                    .decode_message::<v1::SessionCloseRequest>(WireKind::SessionCloseRequest)
                    .expect("decode recorded close")
                    .operation_id
            }
            kind => panic!("unexpected recorded mutation kind: {kind:?}"),
        };
        operation_id
            .expect("recorded mutation contains operation ID")
            .try_into()
            .expect("recorded operation ID is valid")
    }

    fn assert_exact_mutation_replay(
        transport: &CommittedLossTransport,
        request_id: u64,
        expected_request: &[u8],
        expected_operation: OperationId,
    ) {
        let state = transport.state();
        let requests = state
            .requests
            .iter()
            .filter(|(actual, _)| *actual == request_id)
            .map(|(_, bytes)| bytes)
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 2, "one mutation has at most two attempts");
        assert_eq!(requests[0], expected_request);
        assert_eq!(requests[1], expected_request);
        assert_eq!(requests[0], requests[1], "request frames are immutable");
        assert_eq!(operation_id_from_mutation(requests[0]), expected_operation);
        assert_eq!(operation_id_from_mutation(requests[1]), expected_operation);

        let responses = state
            .responses
            .iter()
            .filter(|(actual, _)| *actual == request_id)
            .map(|(_, bytes)| bytes)
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 2);
        assert!(
            responses[0].as_slice() == responses[1].as_slice(),
            "the second host result is the exact cached response"
        );
    }

    fn assert_single_attempt(
        transport: &CommittedLossTransport,
        request_id: u64,
        expected_request: &[u8],
    ) {
        let state = transport.state();
        let requests = state
            .requests
            .iter()
            .filter(|(actual, _)| *actual == request_id)
            .map(|(_, bytes)| bytes.as_slice())
            .collect::<Vec<_>>();
        assert_eq!(requests, [expected_request]);
        assert_eq!(
            state
                .responses
                .iter()
                .filter(|(actual, _)| *actual == request_id)
                .count(),
            1
        );
    }

    fn authorized_registry(
        remote: DeviceId,
        accepted_generation: AuthGeneration,
    ) -> AuthorizationRegistry {
        let authorization = AuthorizationRegistry::new();
        authorization
            .preload(vec![DeviceAuthorization {
                device_id: remote,
                display_name: DeviceDisplayName::new("response-loss controller")
                    .expect("test display name"),
                status: AuthorizationStatus::Authorized,
                generation: accepted_generation,
                paired_at_unix: 1,
                revoked_at_unix: None,
                last_seen_at_unix: None,
            }])
            .expect("preload response-loss authorization");
        authorization
    }

    fn counted_session_service(
        own_device_id: DeviceId,
        working_directory: PathBuf,
        spawn_count: Arc<AtomicUsize>,
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
                spawn_count.fetch_add(1, Ordering::AcqRel);
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

    fn valid_session_summary(byte: u8) -> v1::SessionSummary {
        v1::SessionSummary {
            session_id: Some(v1::SessionId {
                value: vec![byte; 16],
            }),
            name: "build".to_owned(),
            revision: 4,
            has_controller: false,
            working_directory: "/tmp".to_owned(),
            viewport: Some(v1::TerminalViewport {
                rows: 24,
                columns: 80,
            }),
        }
    }

    fn decoded_message<Message: prost::Message>(
        kind: WireKind,
        request_id: u64,
        message: &Message,
    ) -> DecodedFrame {
        let bytes = encode_message(kind, request_id, 0, message).expect("bounded response");
        decode_exact_frame(&bytes).expect("encoded response decodes")
    }
}
