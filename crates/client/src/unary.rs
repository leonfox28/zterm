//! Shared Session unary commands and bounded per-target operation leases.
use crate::{
    error::ClientError,
    model::{ResolvedSessionTarget, SessionSummary, session_summary_from_wire},
    protocol::*,
    transport::TransportFuture,
};
use std::{
    collections::BTreeMap,
    fmt,
    path::Path,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tokio::sync::Mutex as AsyncMutex;
use zterm_core::{DomainErrorKind, OperationId, OperationLease, SessionId, SessionName};
use zterm_proto::{DecodedFrame, WireKind, v2};

const MAX_MUTATION_TARGETS_PER_CLIENT: usize = 64;

/// One logical unary call; the adapter retains pre/post-write retry classification.
pub struct SessionUnaryRequest {
    /// Exact immutable target.
    pub target: ResolvedSessionTarget,
    /// Existing normal-ALPN Session request kind.
    pub request_kind: WireKind,
    /// Required correlated response kind.
    pub response_kind: WireKind,
    /// Encoded semantic message, before framing and adapter correlation.
    pub payload: Vec<u8>,
    /// Control-operation budget.
    pub deadline: Duration,
    /// Stateful calls must preserve operation identity across uncertain delivery.
    pub mutation_or_lease_retry: bool,
}

/// Strict unary exchange adapter, using local IPC or authenticated Iroh service streams.
pub trait SessionUnaryTransport: Send + Sync {
    /// Completes one logical request using route-appropriate ambiguity rules.
    fn request(
        &self,
        request: SessionUnaryRequest,
    ) -> TransportFuture<'_, Result<DecodedFrame, ClientError>>;
}

/// One bounded lease/replay owner shared by all Session commands in a frontend.
pub struct SessionUnaryClient {
    mutation_targets:
        StdMutex<BTreeMap<ResolvedSessionTarget, Arc<AsyncMutex<LocalMutationState>>>>,
}
impl Default for SessionUnaryClient {
    fn default() -> Self {
        let mutation_targets = BTreeMap::from([(
            ResolvedSessionTarget::local(),
            Arc::new(AsyncMutex::new(LocalMutationState {
                lease: None,
                next_sequence: 1,
            })),
        )]);
        Self {
            mutation_targets: StdMutex::new(mutation_targets),
        }
    }
}
struct LocalMutationState {
    lease: Option<OperationLease>,
    next_sequence: u64,
}

impl fmt::Debug for LocalMutationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalMutationState")
            .field("has_lease", &self.lease.is_some())
            .finish_non_exhaustive()
    }
}

impl SessionUnaryClient {
    /// Address-free optional count for redacted diagnostics; never blocks a caller.
    pub fn cached_target_count(&self) -> Option<usize> {
        self.mutation_targets
            .try_lock()
            .ok()
            .map(|targets| targets.len())
    }
    /// Lists live sessions on the local daemon through one strict unary request.
    pub async fn list_sessions(
        &self,
        transport: &dyn SessionUnaryTransport,
    ) -> Result<Vec<SessionSummary>, ClientError> {
        self.list_sessions_at(transport, ResolvedSessionTarget::local())
            .await
    }

    /// Lists live sessions on one already-resolved exact target.
    pub async fn list_sessions_at(
        &self,
        transport: &dyn SessionUnaryTransport,
        target: ResolvedSessionTarget,
    ) -> Result<Vec<SessionSummary>, ClientError> {
        let frame = self
            .session_request(
                transport,
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
    pub async fn create_session(
        &self,
        transport: &dyn SessionUnaryTransport,
        name: &SessionName,
        working_directory: Option<&Path>,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<SessionSummary, ClientError> {
        self.create_session_at(
            transport,
            ResolvedSessionTarget::local(),
            name,
            working_directory,
            viewport,
        )
        .await
    }

    /// Creates a named account-login-shell session on one exact target.
    pub async fn create_session_at(
        &self,
        transport: &dyn SessionUnaryTransport,
        target: ResolvedSessionTarget,
        name: &SessionName,
        working_directory: Option<&Path>,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<SessionSummary, ClientError> {
        self.create_session_at_with_colors(
            transport,
            target,
            name,
            working_directory,
            viewport,
            zterm_core::terminal::TerminalColorProfile::default(),
        )
        .await
    }

    /// Creates a Session with observations available before PTY startup.
    pub async fn create_session_at_with_colors(
        &self,
        transport: &dyn SessionUnaryTransport,
        target: ResolvedSessionTarget,
        name: &SessionName,
        working_directory: Option<&Path>,
        viewport: Option<zterm_core::terminal::TerminalSize>,
        base_colors: zterm_core::terminal::TerminalColorProfile,
    ) -> Result<SessionSummary, ClientError> {
        let frame = self
            .mutation_request(
                transport,
                target,
                WireKind::SessionCreateRequest,
                |operation_id| v2::SessionCreateRequest {
                    base_colors: Some(base_colors.clone().into()),
                    operation_id: Some(operation_id.into()),
                    target: Some(resolved_target_wire(target)),
                    name: name.to_string(),
                    working_directory: working_directory
                        .map_or_else(String::new, |path| path.to_string_lossy().into_owned()),
                    viewport: viewport.map(Into::into),
                },
            )
            .await?;
        mutate_response(frame)
    }

    /// Renames a live session without changing its identity.
    pub async fn rename_session(
        &self,
        transport: &dyn SessionUnaryTransport,
        session_id: SessionId,
        name: &SessionName,
    ) -> Result<SessionSummary, ClientError> {
        self.rename_session_at(transport, ResolvedSessionTarget::local(), session_id, name)
            .await
    }

    /// Renames a live session on one exact target without changing its identity.
    pub async fn rename_session_at(
        &self,
        transport: &dyn SessionUnaryTransport,
        target: ResolvedSessionTarget,
        session_id: SessionId,
        name: &SessionName,
    ) -> Result<SessionSummary, ClientError> {
        let frame = self
            .mutation_request(
                transport,
                target,
                WireKind::SessionRenameRequest,
                |operation_id| v2::SessionRenameRequest {
                    operation_id: Some(operation_id.into()),
                    target: Some(resolved_target_wire(target)),
                    session_id: Some(session_id.into()),
                    name: name.to_string(),
                },
            )
            .await?;
        mutate_response(frame)
    }

    /// Explicitly closes one live session.
    pub async fn close_session(
        &self,
        transport: &dyn SessionUnaryTransport,
        session_id: SessionId,
    ) -> Result<SessionSummary, ClientError> {
        self.close_session_at(transport, ResolvedSessionTarget::local(), session_id)
            .await
    }

    /// Explicitly closes one live session on an exact target.
    pub async fn close_session_at(
        &self,
        transport: &dyn SessionUnaryTransport,
        target: ResolvedSessionTarget,
        session_id: SessionId,
    ) -> Result<SessionSummary, ClientError> {
        let frame = self
            .mutation_request(
                transport,
                target,
                WireKind::SessionCloseRequest,
                |operation_id| v2::SessionCloseRequest {
                    operation_id: Some(operation_id.into()),
                    target: Some(resolved_target_wire(target)),
                    session_id: Some(session_id.into()),
                },
            )
            .await?;
        mutate_response(frame)
    }

    async fn mutation_request<Message, Build>(
        &self,
        transport: &dyn SessionUnaryTransport,
        target: ResolvedSessionTarget,
        request_kind: WireKind,
        build: Build,
    ) -> Result<DecodedFrame, ClientError>
    where
        Message: prost::Message,
        Build: FnOnce(OperationId) -> Message,
    {
        // Only one exact target is serialized. No remote await holds the map
        // mutex or blocks local/other-device lease streams.
        let state = self.mutation_target_state(target)?;
        let mut mutation = state.lock().await;
        if mutation.lease.is_none() {
            mutation.lease = Some(self.issue_operation_lease(transport, target).await?);
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
                transport,
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

    async fn issue_operation_lease(
        &self,
        transport: &dyn SessionUnaryTransport,
        target: ResolvedSessionTarget,
    ) -> Result<OperationLease, ClientError> {
        let frame = self
            .session_request(
                transport,
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

    fn mutation_target_state(
        &self,
        target: ResolvedSessionTarget,
    ) -> Result<Arc<AsyncMutex<LocalMutationState>>, ClientError> {
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

    #[allow(clippy::too_many_arguments)]
    async fn session_request<Message: prost::Message>(
        &self,
        transport: &dyn SessionUnaryTransport,
        target: ResolvedSessionTarget,
        request_kind: WireKind,
        response_kind: WireKind,
        message: &Message,
        deadline: Duration,
        mutation_or_lease_retry: bool,
    ) -> Result<DecodedFrame, ClientError> {
        transport
            .request(SessionUnaryRequest {
                target,
                request_kind,
                response_kind,
                payload: message.encode_to_vec(),
                deadline,
                mutation_or_lease_retry,
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zterm_core::{DaemonIncarnation, DeviceId};
    #[test]
    fn mutation_state_debug_redacts_private_lease_owner() {
        let mutation = LocalMutationState {
            lease: Some(OperationLease {
                daemon_incarnation: DaemonIncarnation::from_array(*b"LEASE_SENTINEL__"),
                ordinal: 8_675_309,
            }),
            next_sequence: 2_434_117,
        };
        let debug = format!("{mutation:?}");
        for sentinel in ["LEASE_SENTINEL__", "8675309", "2434117"] {
            assert!(!debug.contains(sentinel));
        }
        assert!(debug.contains("has_lease: true"));
    }
    #[tokio::test]
    async fn mutation_lease_state_is_isolated_and_serialized_only_per_exact_target() {
        let client = SessionUnaryClient::default();
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
        let client = SessionUnaryClient::default();
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

        let saturated = SessionUnaryClient::default();
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
