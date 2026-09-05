//! Same-UID unary clients; daemon lifecycle remains with the command runtime.
#[cfg(unix)]
use super::{
    DEFAULT_DEADLINE, connect_error, daemon_io, decode_response, malformed, resolved_target_wire,
    resource_error, service_error,
};
use crate::{
    config::ValidatedConfig,
    device_directory::ResolvedSessionTarget,
    error::DaemonError,
    service::{DaemonReadiness, DaemonStatus, SessionImpact, ValidatedSetupStatus},
};
#[cfg(unix)]
use crate::{
    network::{AddressServiceState, NetworkDiagnostic, NetworkObservation, NetworkState},
    pairing::PairTicketText,
    remote_session::{
        SessionUnaryResponseStatus, session_summary_from_wire, validate_session_unary_response,
    },
    service::{ProtocolStatus, protocol_error},
};
#[cfg(unix)]
use ring::rand::{SecureRandom, SystemRandom};
#[cfg(unix)]
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use std::{
    fmt,
    path::{Path, PathBuf},
};
#[cfg(unix)]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex as AsyncMutex,
};
#[cfg(unix)]
use zeroize::{Zeroize, Zeroizing};
#[cfg(unix)]
use zterm_core::{
    DEFAULT_PAIR_TTL_SECONDS, EphemeralOperationId, OperationId, OperationLease, PairFingerprint,
};
use zterm_core::{DeviceAlias, DeviceId, DeviceSummary, DomainErrorKind, SessionId, SessionName};
#[cfg(unix)]
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v2};
#[cfg(unix)]
const PAIRING_DEADLINE: Duration = Duration::from_secs(15);
#[cfg(unix)]
const MAX_MUTATION_TARGETS_PER_CLIENT: usize = 64;

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
                &v2::LocalReadinessRequest {},
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v2::LocalReadinessResponse = decode_response(&frame)?;
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
                &v2::LocalStatusRequest {},
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v2::LocalStatusResponse = decode_response(&frame)?;
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
                &v2::LocalValidateSetupRequest {
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
        let response: v2::LocalValidateSetupResponse = decode_response(&frame)?;
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
                &v2::LocalStopRequest {
                    force,
                    operation_id: None,
                },
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v2::LocalStopResponse = decode_response(&frame)?;
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
                &v2::LocalUpdatePreflightRequest {},
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v2::LocalUpdatePreflightResponse = decode_response(&frame)?;
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
                &v2::LocalTargetResolveRequest {
                    selector: selector.to_owned(),
                },
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v2::LocalTargetResolveResponse = decode_response(&frame)?;
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
                &v2::SessionListRequest {
                    target: Some(resolved_target_wire(target)),
                },
                DEFAULT_DEADLINE,
                false,
            )
            .await?;
        let response: v2::SessionListResponse = decode_response(&frame)?;
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
                v2::SessionCreateRequest {
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
                v2::SessionRenameRequest {
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
                v2::SessionCloseRequest {
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
                &v2::SessionOperationLeaseRequest {
                    target: Some(resolved_target_wire(target)),
                },
                DEFAULT_DEADLINE,
                true,
            )
            .await?;
        let response: v2::SessionOperationLeaseResponse = decode_response(&frame)?;
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
        let mut envelope = v2::LocalSessionUnaryRequest {
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
        mut message: v2::LocalPairAcceptRequest,
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
                &v2::LocalDeviceListRequest {},
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v2::LocalDeviceListResponse = decode_response(&frame)?;
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
                &v2::LocalDeviceRenameRequest {
                    device_id: Some(device_id.into()),
                    alias: alias.as_str().to_owned(),
                },
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v2::LocalDeviceRenameResponse = decode_response(&frame)?;
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
                &v2::LocalDeviceRevokeRequest {
                    device_id: Some(device_id.into()),
                },
                DEFAULT_DEADLINE,
            )
            .await?;
        let response: v2::LocalDeviceRevokeResponse = decode_response(&frame)?;
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
                &v2::LocalPairCreateRequest {
                    ephemeral_operation_id: operation_id.as_bytes().to_vec(),
                    fingerprint: fingerprint.as_bytes().to_vec(),
                    ttl_seconds,
                },
                PAIRING_DEADLINE,
            )
            .await?;
        let response = decode_response::<v2::LocalPairCreateResponse>(&frame);
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
        let request = v2::LocalPairAcceptRequest {
            ephemeral_operation_id: operation_id.as_bytes().to_vec(),
            fingerprint: fingerprint.as_bytes().to_vec(),
            ticket: ticket.expose().to_owned(),
            alias: alias.map_or_else(String::new, |alias| alias.as_str().to_owned()),
        };
        let result = self.client.request_pair_accept(request).await;
        drop(ticket);
        let mut frame = result?;
        let response = decode_response::<v2::LocalPairAcceptResponse>(&frame);
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
fn protocol_status(protocol: Option<v2::ProtocolVersion>) -> Result<ProtocolStatus, DaemonError> {
    let protocol = protocol.ok_or_else(|| malformed("local response omitted protocol"))?;
    Ok(ProtocolStatus {
        wire_major: protocol.wire_major,
        state_schema: protocol.state_schema,
        capabilities: protocol.capabilities,
    })
}

#[cfg(unix)]
fn network_observation(
    response: &v2::LocalStatusResponse,
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
pub(super) fn mutate_response(
    frame: DecodedFrame,
) -> Result<crate::session::SessionSummary, DaemonError> {
    let response: v2::SessionMutateResponse = decode_response(&frame)?;
    session_summary_from_wire(
        response
            .session
            .ok_or_else(|| malformed("session mutation response omitted session"))?,
    )
}

#[cfg(unix)]
pub(super) fn resolved_target_from_wire(
    target: Option<v2::TargetSelector>,
) -> Result<ResolvedSessionTarget, DaemonError> {
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

#[cfg(not(unix))]
fn unsupported() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::UnsupportedPlatform,
        "local daemon IPC is Unix-only in the current milestone",
    )
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use zterm_core::DaemonIncarnation;
    #[test]
    fn local_client_and_mutation_state_debug_redact_private_owners() {
        let client = LocalClient::new("/private/tmp/SOCKET_SENTINEL");
        let mutation = LocalMutationState {
            lease: Some(OperationLease {
                daemon_incarnation: DaemonIncarnation::from_array(*b"LEASE_SENTINEL__"),
                ordinal: 8_675_309,
            }),
            next_sequence: 2_434_117,
        };
        let debug = format!("{client:?} {mutation:?}");
        for sentinel in ["SOCKET_SENTINEL", "LEASE_SENTINEL__", "8675309", "2434117"] {
            assert!(!debug.contains(sentinel));
        }
        assert!(debug.contains("mutation_target_count: Some(1)"));
        assert!(debug.contains("has_lease: true"));
    }

    #[tokio::test]
    async fn remote_mutation_outer_malformed_or_truncated_reply_sends_once() {
        let request_id = 501;
        let target = DeviceId::from_array([0xa1; DeviceId::LENGTH]);
        let mut truncated = encode_message(
            WireKind::SessionMutateResponse,
            request_id,
            0,
            &v2::SessionMutateResponse {
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
            &v2::SessionListResponse { sessions: vec![] },
        )
        .expect("bounded wrong-kind reply");
        let wrong_id = encode_message(
            WireKind::SessionMutateResponse,
            request_id + 1,
            0,
            &v2::SessionMutateResponse {
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
            &v2::SessionMutateResponse { session: None },
        )
        .expect("well-framed incomplete mutation response");
        let unknown_error_code = encode_message(
            WireKind::ServiceErrorResponse,
            request_id,
            0,
            &v2::ServiceError {
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
            &v2::SessionListResponse { sessions: vec![] },
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
            &v2::SessionListRequest {
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
            &v2::SessionOperationLeaseRequest {
                target: Some(resolved_target_wire(ResolvedSessionTarget::device(target))),
            },
        )
        .expect("bounded remote lease request");
        let valid_second_response = encode_message(
            WireKind::SessionOperationLeaseResponse,
            request_id,
            0,
            &v2::SessionOperationLeaseResponse {
                lease: Some(v2::OperationLease {
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
            &v2::SessionMutateResponse {
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

    fn fake_remote_create_request(target: DeviceId, request_id: u64) -> Vec<u8> {
        encode_message(
            WireKind::SessionCreateRequest,
            request_id,
            1_000,
            &v2::SessionCreateRequest {
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

    fn fake_session_summary(byte: u8) -> v2::SessionSummary {
        v2::SessionSummary {
            session_id: Some(v2::SessionId {
                value: vec![byte; SessionId::LENGTH],
            }),
            name: "outer-ambiguity".to_owned(),
            revision: 2,
            has_controller: false,
            working_directory: "/tmp".to_owned(),
            viewport: Some(v2::TerminalViewport {
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
