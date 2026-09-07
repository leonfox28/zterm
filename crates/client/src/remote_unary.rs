//! Authenticated remote unary framing, correlation and exact-byte retry policy.
use crate::{
    error::ClientError as DaemonError,
    model::session_summary_from_wire,
    protocol::{malformed, protocol_error},
};
use prost::Message;
use std::{future::Future, pin::Pin, sync::Arc, time::Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use zeroize::Zeroize;
use zterm_core::{DeviceId, DomainErrorKind, OperationLease};
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, v2};
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Content-free remote failure text; remote diagnostics never cross this boundary.
pub const REMOTE_SESSION_FAILURE_DETAIL: &str = "remote Session request failed";

/// One logical outbound unary client. It acquires one demand and keeps it for
/// both possible service-stream attempts.
#[derive(Clone)]
pub struct RemoteUnaryClient {
    transport: Arc<dyn RemoteUnaryTransport>,
}

impl RemoteUnaryClient {
    /// Composes the single logical retry owner with an authenticated adapter.
    pub fn new(transport: Arc<dyn RemoteUnaryTransport>) -> Self {
        Self { transport }
    }

    /// Inspects the exact request bytes before retaining them for any retry.
    pub async fn execute_preencoded(
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

    /// Executes an already validated immutable request under one absolute deadline.
    pub async fn execute_validated(
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
/// Validated target, response kind and replay class for immutable request bytes.
pub struct RequestContract {
    request_id: u64,
    response_kind: WireKind,
    retry_class: RetryClass,
}

impl RequestContract {
    /// Validates exactly one allowed Session frame and its immutable device target.
    pub fn inspect(request: &[u8], target: DeviceId) -> Result<Self, DaemonError> {
        let frame = decode_exact_frame(request)?;
        let (wire_target, response_kind, retry_class) = match frame.kind {
            WireKind::SessionListRequest => {
                let message: v2::SessionListRequest = frame
                    .decode_message(WireKind::SessionListRequest)
                    .map_err(protocol_error)?;
                (
                    message.target,
                    WireKind::SessionListResponse,
                    RetryClass::Safe,
                )
            }
            WireKind::SessionOperationLeaseRequest => {
                let message: v2::SessionOperationLeaseRequest = frame
                    .decode_message(WireKind::SessionOperationLeaseRequest)
                    .map_err(protocol_error)?;
                (
                    message.target,
                    WireKind::SessionOperationLeaseResponse,
                    RetryClass::StatefulControl,
                )
            }
            WireKind::SessionCreateRequest => {
                let message: v2::SessionCreateRequest = frame
                    .decode_message(WireKind::SessionCreateRequest)
                    .map_err(protocol_error)?;
                (
                    message.target,
                    WireKind::SessionMutateResponse,
                    RetryClass::Mutation,
                )
            }
            WireKind::SessionRenameRequest => {
                let message: v2::SessionRenameRequest = frame
                    .decode_message(WireKind::SessionRenameRequest)
                    .map_err(protocol_error)?;
                (
                    message.target,
                    WireKind::SessionMutateResponse,
                    RetryClass::Mutation,
                )
            }
            WireKind::SessionCloseRequest => {
                let message: v2::SessionCloseRequest = frame
                    .decode_message(WireKind::SessionCloseRequest)
                    .map_err(protocol_error)?;
                (
                    message.target,
                    WireKind::SessionMutateResponse,
                    RetryClass::Mutation,
                )
            }
            WireKind::SessionTakeoverRequest => {
                let message: v2::SessionTakeoverRequest = frame
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

    /// Requires outer/inner correlation to identify the same operation.
    pub fn require_request_id(self, request_id: u64) -> Result<(), DaemonError> {
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

/// A validated unary result, with a content-free peer error projection.
pub enum SessionUnaryResponseStatus {
    /// Correlated expected kind with a validated typed payload.
    Expected,
    /// Correlated, bounded, known domain failure.
    ServiceError(DaemonError),
}

/// Validates correlation, message kind and the typed response payload.
pub fn validate_session_unary_response(
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

/// Clears untrusted remote diagnostic content while retaining its stable category.
pub fn decode_session_service_error(frame: &DecodedFrame) -> Result<DaemonError, DaemonError> {
    let mut response: v2::ServiceError = frame
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
    let response = v2::ServiceError {
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
            let response: v2::SessionListResponse = frame
                .decode_message(expected_kind)
                .map_err(protocol_error)?;
            for summary in response.sessions {
                session_summary_from_wire(summary)?;
            }
            Ok(())
        }
        WireKind::SessionOperationLeaseResponse => {
            let response: v2::SessionOperationLeaseResponse = frame
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
            let response: v2::SessionMutateResponse = frame
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

/// Acquires one authenticated demand for both possible stream attempts.
pub trait RemoteUnaryTransport: Send + Sync {
    /// Acquires a bounded authenticated connection demand for an exact host.
    fn demand<'a>(
        &'a self,
        target: DeviceId,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Box<dyn RemoteUnaryDemand>, DaemonError>>;
}

/// Connection demand retained across retries of the same immutable bytes.
pub trait RemoteUnaryDemand: Send {
    /// Exchanges one frame and classifies failure relative to the first write.
    fn exchange<'a>(
        &'a mut self,
        request: &'a [u8],
        deadline: Instant,
    ) -> BoxFuture<'a, Result<DecodedFrame, RemoteAttemptError>>;
}

/// Transport delivery classification used by the single replay owner.
pub enum RemoteAttemptError {
    /// Nothing was submitted on this attempt.
    PreWrite(DaemonError),
    /// A remote commit may have occurred, including partial writes.
    PostWrite(DaemonError),
}

/// Reads exactly one complete response and EOF under the absolute deadline.
pub async fn read_exact_response<Reader>(
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

/// Validates a buffer as exactly one complete protocol frame.
pub fn decode_exact_frame(bytes: &[u8]) -> Result<DecodedFrame, DaemonError> {
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
    target: Option<v2::TargetSelector>,
    expected: DeviceId,
) -> Result<(), DaemonError> {
    let Some(v2::target_selector::Target::Device(device)) = target.and_then(|target| target.target)
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

/// Applies a remaining absolute deadline without extending it across retries.
pub async fn timeout_until<T>(
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

fn outcome_unknown() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::OperationOutcomeUnknown,
        "remote Session mutation may have committed but no complete response was received",
    )
}

fn transport_unavailable(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::TransportUnavailable, detail)
}
