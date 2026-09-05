//! Runtime orchestration for one-time pairing over the dedicated pair ALPN.
//!
//! [`crate::pairing::PairingManager`] remains the sole offer/verifier state
//! owner. This adapter composes it with the StoreActor, authorization gate,
//! shared device directory, and daemon-owned connection broker while keeping
//! every ticket/proof buffer redacted and every operation bounded.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::future::Future;
use std::io;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use futures_util::FutureExt;
use iroh::endpoint::{RecvStream, SendStream};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use zeroize::{Zeroize, Zeroizing};
use zterm_core::{
    AuthGeneration, AuthorizationSnapshot, AuthorizationStatus, DEFAULT_PAIR_TTL_SECONDS,
    DeviceAlias, DeviceDisplayName, DeviceId, DomainErrorKind, EphemeralOperationId,
    MAX_TICKET_TEXT_BYTES, PAIR_PROTOCOL_VERSION, PairBegin, PairChallenge, PairFingerprint,
    PairNonce, PairProof, PairSecret, PairTicketFields, RelayHint, TransportLimits,
};
use zterm_proto::{WireKind, v2};

use crate::authorization::AuthorizationRegistry;
use crate::connection_broker::{ConnectionBroker, ConnectionIdentity, PairConnection};
use crate::device_directory::DeviceDirectory;
use crate::error::DaemonError;
use crate::network::{
    NetworkObservation, NetworkObserver, NetworkState, PairConnectionHandler,
    PairConnectionHandlerFuture,
};
use crate::pair_framing::PairFraming;
use crate::pairing::{
    PairConsumption, PairOfferCreated, PairOfferRequest, PairingError, PairingManager,
    controller_transcript,
};
use crate::store::{RelayRouteCache, StoreHandle};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type AcceptOutcome = Result<PairAcceptResult, DaemonError>;

/// Validated local input for one pair-create operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalPairCreateInput {
    operation_id: EphemeralOperationId,
    fingerprint: PairFingerprint,
    requested_ttl_seconds: u64,
}

impl LocalPairCreateInput {
    /// Constructs the server input after fixed-width wire fields are decoded.
    #[must_use]
    pub fn new(
        operation_id: EphemeralOperationId,
        fingerprint: PairFingerprint,
        requested_ttl_seconds: u64,
    ) -> Self {
        Self {
            operation_id,
            fingerprint,
            requested_ttl_seconds,
        }
    }
}

/// Secret-bearing local input for one pair-accept operation.
pub struct LocalPairAcceptInput {
    operation_id: EphemeralOperationId,
    fingerprint: PairFingerprint,
    ticket: Zeroizing<String>,
    explicit_alias: Option<DeviceAlias>,
}

impl LocalPairAcceptInput {
    /// Takes ownership of ticket text immediately after protobuf decoding.
    #[must_use]
    pub fn new(
        operation_id: EphemeralOperationId,
        fingerprint: PairFingerprint,
        ticket: String,
        explicit_alias: Option<DeviceAlias>,
    ) -> Self {
        Self {
            operation_id,
            fingerprint,
            ticket: Zeroizing::new(ticket),
            explicit_alias,
        }
    }
}

impl fmt::Debug for LocalPairAcceptInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalPairAcceptInput")
            .field("operation_id", &self.operation_id)
            .field("fingerprint", &self.fingerprint)
            .field("ticket", &"[REDACTED]")
            .field("explicit_alias", &self.explicit_alias)
            .finish()
    }
}

/// Directional result after normal ALPN confirmation and local persistence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairAcceptResult {
    device_id: DeviceId,
    alias: DeviceAlias,
    remote_name: DeviceDisplayName,
    authorization_generation: AuthGeneration,
    verified_relay: Option<RelayHint>,
}

impl PairAcceptResult {
    /// Host identity learned from the ticket and rechecked by TLS/normal ALPN.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Locally reserved and durably committed alias.
    #[must_use]
    pub fn alias(&self) -> &DeviceAlias {
        &self.alias
    }

    /// Host display name carried by the validated ticket.
    #[must_use]
    pub fn remote_name(&self) -> &DeviceDisplayName {
        &self.remote_name
    }

    /// Receiver-owned generation proven by the normal Welcome.
    #[must_use]
    pub const fn authorization_generation(&self) -> AuthGeneration {
        self.authorization_generation
    }

    /// Relay route which survived normal TLS/application authentication.
    #[must_use]
    pub fn verified_relay(&self) -> Option<&RelayHint> {
        self.verified_relay.as_ref()
    }
}

/// Cloneable runtime pairing coordinator installed into local IPC and the
/// network pair-ALPN callback.
#[derive(Clone)]
pub struct PairingService {
    inner: Arc<PairingServiceInner>,
}

struct PairingServiceInner {
    manager: PairingManager,
    store: StoreHandle,
    authorization: AuthorizationRegistry,
    directory: DeviceDirectory,
    transport: Arc<dyn PairTransport>,
    network: Arc<dyn NetworkStatusSource>,
    identity: ConnectionIdentity,
    diagnostic_version: DeviceDisplayName,
    limits: TransportLimits,
    accepts: Mutex<AcceptRegistry>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    shutdown: watch::Sender<bool>,
    #[cfg(test)]
    authorize_faults: Mutex<VecDeque<AuthorizeFault>>,
}

impl PairingService {
    /// Composes existing daemon owners without binding another Endpoint or
    /// creating another directory/store registry.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        manager: PairingManager,
        store: StoreHandle,
        authorization: AuthorizationRegistry,
        directory: DeviceDirectory,
        broker: ConnectionBroker,
        network: NetworkObserver,
        identity: ConnectionIdentity,
        limits: TransportLimits,
    ) -> Result<Self, DaemonError> {
        Self::with_dependencies(
            manager,
            store,
            authorization,
            directory,
            Arc::new(BrokerPairTransport { broker }),
            Arc::new(network),
            identity,
            limits,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_dependencies(
        manager: PairingManager,
        store: StoreHandle,
        authorization: AuthorizationRegistry,
        directory: DeviceDirectory,
        transport: Arc<dyn PairTransport>,
        network: Arc<dyn NetworkStatusSource>,
        identity: ConnectionIdentity,
        limits: TransportLimits,
    ) -> Result<Self, DaemonError> {
        limits.validate().map_err(|error| {
            DaemonError::new(DomainErrorKind::ResourceExhausted, error.to_string())
        })?;
        if manager.host_device_id() != identity.device_id() {
            return Err(DaemonError::new(
                DomainErrorKind::IdentityStateMismatch,
                "pairing manager identity does not match the connection broker identity",
            ));
        }
        let diagnostic_version = DeviceDisplayName::new(identity.build()).map_err(|error| {
            DaemonError::new(
                DomainErrorKind::IdentityInvalid,
                format!("invalid pairing diagnostic version: {error}"),
            )
        })?;
        let (shutdown, _) = watch::channel(false);
        Ok(Self {
            inner: Arc::new(PairingServiceInner {
                manager,
                store,
                authorization,
                directory,
                transport,
                network,
                identity,
                diagnostic_version,
                limits,
                accepts: Mutex::new(AcceptRegistry::default()),
                tasks: Mutex::new(Vec::new()),
                shutdown,
                #[cfg(test)]
                authorize_faults: Mutex::new(VecDeque::new()),
            }),
        })
    }

    /// Creates or exactly replays one local ticket mutation.
    pub fn create_until(
        &self,
        input: LocalPairCreateInput,
        deadline: Instant,
    ) -> Result<PairOfferCreated, DaemonError> {
        self.inner.create_until(input, deadline)
    }

    /// Starts or joins one bounded local pair-accept operation. The operation
    /// task retains ownership after this waiter times out.
    pub async fn accept_until(
        &self,
        input: LocalPairAcceptInput,
        deadline: Instant,
    ) -> Result<PairAcceptResult, DaemonError> {
        self.inner.clone().accept_until(input, deadline).await
    }

    /// Handles one fully TLS-authenticated inbound pair connection.
    pub async fn accept_pair_connection(
        &self,
        connection: PairConnection,
        deadline: Instant,
    ) -> Result<(), DaemonError> {
        self.inner
            .clone()
            .accept_pair_connection(connection, deadline)
            .await
    }

    /// Cancels new/running local accept operations and joins every owned task
    /// within the caller's one absolute shutdown deadline.
    pub async fn shutdown_until(&self, deadline: Instant) -> Result<(), DaemonError> {
        self.inner.shutdown.send_replace(true);
        let mut tasks = {
            let mut owned = mutex_lock(&self.inner.tasks);
            std::mem::take(&mut *owned)
        };
        while let Some(mut task) = tasks.pop() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                task.abort();
                tasks.iter().for_each(JoinHandle::abort);
                return Err(deadline_exceeded("pairing shutdown deadline elapsed"));
            }
            match tokio::time::timeout(remaining, &mut task).await {
                Ok(_) => {}
                Err(_) => {
                    task.abort();
                    tasks.iter().for_each(JoinHandle::abort);
                    return Err(deadline_exceeded("pairing shutdown deadline elapsed"));
                }
            }
        }
        Ok(())
    }
}

impl PairingServiceInner {
    fn create_until(
        &self,
        input: LocalPairCreateInput,
        deadline: Instant,
    ) -> Result<PairOfferCreated, DaemonError> {
        self.ensure_running()?;
        ensure_deadline(deadline, "local pair create deadline elapsed")?;

        let effective_ttl_seconds = if input.requested_ttl_seconds == 0 {
            DEFAULT_PAIR_TTL_SECONDS
        } else {
            input.requested_ttl_seconds
        };
        let expected_fingerprint = PairFingerprint::for_create(effective_ttl_seconds);
        if input.fingerprint != expected_fingerprint {
            return Err(pair_outcome_unknown(
                "pair create operation fingerprint did not match its arguments",
            ));
        }

        let observation = self.network.snapshot();
        if observation.device_id != self.identity.device_id() {
            return Err(DaemonError::new(
                DomainErrorKind::IdentityStateMismatch,
                "network observation identity does not match pairing identity",
            ));
        }
        if observation.state != NetworkState::Online || !observation.endpoint_bound {
            return Err(DaemonError::new(
                DomainErrorKind::AddressUnavailable,
                "pair ticket creation requires an online bound endpoint",
            ));
        }
        let home_relay = observation
            .home_relay
            .filter(|relay| !relay.is_empty())
            .ok_or_else(|| {
                DaemonError::new(
                    DomainErrorKind::AddressUnavailable,
                    "pair ticket creation requires a current home relay",
                )
            })?;
        let home_relay = RelayHint::new(home_relay).map_err(|_| {
            DaemonError::new(
                DomainErrorKind::AddressUnavailable,
                "the current home relay is not a valid pairing route",
            )
        })?;
        let host_name =
            DeviceDisplayName::new(self.identity.display_name().to_owned()).map_err(|_| {
                DaemonError::new(
                    DomainErrorKind::IdentityStateMismatch,
                    "pairing identity has an invalid display name",
                )
            })?;
        let request = PairOfferRequest::new(
            input.operation_id,
            input.fingerprint,
            host_name,
            vec![home_relay],
            effective_ttl_seconds,
        )
        .map_err(DaemonError::from)?;
        self.manager
            .create_offer_until(request, deadline)
            .map_err(DaemonError::from)
    }

    async fn accept_until(
        self: Arc<Self>,
        input: LocalPairAcceptInput,
        deadline: Instant,
    ) -> AcceptOutcome {
        let deadline = self.cap_pairing_deadline(deadline)?;
        self.ensure_running()?;
        ensure_deadline(deadline, "local pair accept deadline elapsed")?;
        if input.ticket.len() > self.limits.max_ticket_text_bytes
            || input.ticket.len() > MAX_TICKET_TEXT_BYTES
        {
            return Err(DaemonError::new(
                DomainErrorKind::PairTicketInvalid,
                "pairing ticket exceeds the configured text limit",
            ));
        }
        let expected_fingerprint =
            PairFingerprint::for_accept(input.ticket.as_bytes(), input.explicit_alias.as_ref());
        if input.fingerprint != expected_fingerprint {
            return Err(pair_outcome_unknown(
                "pair accept operation fingerprint did not match its arguments",
            ));
        }

        if let Some(replay) = self.existing_accept(input.operation_id, &input.fingerprint)? {
            return match replay {
                AcceptReplay::Live(cell) => {
                    cell.wait_until(deadline, self.shutdown.subscribe()).await
                }
                AcceptReplay::Complete(result) => result,
            };
        }

        let (fields, secret) = zterm_proto::decode_pair_ticket(input.ticket.as_str())
            .map_err(|_| invalid_pair_ticket("pairing ticket text is invalid"))?;
        let now_unix = unix_now_u64()?;
        if fields.is_expired(now_unix) {
            return Err(DaemonError::new(
                DomainErrorKind::PairTicketExpired,
                "pairing ticket has expired",
            ));
        }
        if fields.host_device_id() == self.identity.device_id() {
            return Err(invalid_pair_ticket("a device cannot pair with itself"));
        }
        iroh::EndpointId::from_bytes(fields.host_device_id().as_bytes())
            .map_err(|_| invalid_pair_ticket("pairing ticket host identity is invalid"))?;
        let remote_name = DeviceDisplayName::new(fields.host_name().to_owned())
            .map_err(|_| invalid_pair_ticket("pairing ticket host name is invalid"))?;
        let prepared = PendingAccept {
            fields,
            secret,
            remote_name,
            explicit_alias: input.explicit_alias,
        };

        let cell = match self.admit_accept(input.operation_id, &input.fingerprint)? {
            AcceptAdmission::Created(cell) => cell,
            AcceptAdmission::Live(cell) => {
                drop(prepared);
                return cell.wait_until(deadline, self.shutdown.subscribe()).await;
            }
            AcceptAdmission::Complete(result) => {
                drop(prepared);
                return result;
            }
        };
        let completed_during_shutdown = if *self.shutdown.borrow() {
            let result = Err(cancelled("pairing service is stopping"));
            cell.complete(result);
            true
        } else {
            let operation_id = input.operation_id;
            let operation_cell = Arc::clone(&cell);
            let operation_inner = Arc::clone(&self);
            let task = tokio::spawn(async move {
                let result = AssertUnwindSafe(
                    Arc::clone(&operation_inner).execute_accept(prepared, deadline),
                )
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(pair_outcome_unknown(
                        "pair accept operation ended unexpectedly",
                    ))
                });
                match &result {
                    Ok(_) => tracing::info!(
                        component = "pairing",
                        operation = "accept_committed",
                        "Outbound pairing committed"
                    ),
                    Err(error) => tracing::warn!(
                        component = "pairing",
                        operation = "accept_failed",
                        reason = error.kind().code(),
                        "Pair acceptance failed"
                    ),
                }
                operation_cell.complete(result);
                operation_inner.record_accept_completion(operation_id, &operation_cell);
            });
            let mut tasks = mutex_lock(&self.tasks);
            tasks.retain(|task| !task.is_finished());
            tasks.push(task);
            false
        };
        if completed_during_shutdown {
            self.record_accept_completion(input.operation_id, &cell);
        }

        cell.wait_until(deadline, self.shutdown.subscribe()).await
    }

    async fn accept_pair_connection(
        self: Arc<Self>,
        connection: PairConnection,
        deadline: Instant,
    ) -> Result<(), DaemonError> {
        let deadline = self.cap_pairing_deadline(deadline)?;
        self.ensure_running()?;
        ensure_deadline(deadline, "inbound pairing handshake deadline elapsed")?;
        if connection.local() != self.identity.device_id() {
            return Err(DaemonError::new(
                DomainErrorKind::IdentityStateMismatch,
                "inbound pairing connection has the wrong local identity",
            ));
        }
        if connection.remote() == connection.local() {
            return Err(DaemonError::new(
                DomainErrorKind::Unauthorized,
                "a device cannot pair with itself",
            ));
        }
        let io = BrokerPairIo::accept(connection, deadline).await?;
        self.run_host(Box::new(io), deadline).await
    }

    fn ensure_running(&self) -> Result<(), DaemonError> {
        if *self.shutdown.borrow() {
            Err(cancelled("pairing service is stopping"))
        } else {
            Ok(())
        }
    }

    fn cap_pairing_deadline(&self, caller_deadline: Instant) -> Result<Instant, DaemonError> {
        let now = Instant::now();
        let service_deadline = now
            .checked_add(self.limits.pairing_total_deadline)
            .ok_or_else(|| deadline_exceeded("pairing deadline arithmetic overflowed"))?;
        let deadline = caller_deadline.min(service_deadline);
        if now >= deadline {
            Err(deadline_exceeded("pairing operation deadline elapsed"))
        } else {
            Ok(deadline)
        }
    }

    fn admit_accept(
        &self,
        operation_id: EphemeralOperationId,
        fingerprint: &PairFingerprint,
    ) -> Result<AcceptAdmission, DaemonError> {
        let mut registry = mutex_lock(&self.accepts);
        if let Some(cell) = registry.cells.get(&operation_id) {
            return if cell.fingerprint == *fingerprint {
                Ok(AcceptAdmission::Live(Arc::clone(cell)))
            } else {
                Err(pair_outcome_unknown(
                    "pair accept operation ID was reused for different arguments",
                ))
            };
        }
        if let Some(retired) = registry.retired.get(&operation_id) {
            return if retired.fingerprint == *fingerprint {
                Ok(AcceptAdmission::Complete(retired.result.clone()))
            } else {
                Err(pair_outcome_unknown(
                    "retired pair accept operation ID was reused for different arguments",
                ))
            };
        }

        while registry.cells.len() >= self.limits.max_live_pair_offers {
            if !retire_oldest_accept(&mut registry, self.limits.max_live_pair_offers) {
                return Err(DaemonError::new(
                    DomainErrorKind::ResourceExhausted,
                    "pair accept operation capacity is exhausted",
                ));
            }
        }

        let cell = Arc::new(AcceptCell::new(fingerprint.clone()));
        registry.cells.insert(operation_id, Arc::clone(&cell));
        Ok(AcceptAdmission::Created(cell))
    }

    fn existing_accept(
        &self,
        operation_id: EphemeralOperationId,
        fingerprint: &PairFingerprint,
    ) -> Result<Option<AcceptReplay>, DaemonError> {
        let registry = mutex_lock(&self.accepts);
        match registry.cells.get(&operation_id) {
            Some(cell) if cell.fingerprint == *fingerprint => {
                Ok(Some(AcceptReplay::Live(Arc::clone(cell))))
            }
            Some(_) => Err(pair_outcome_unknown(
                "pair accept operation ID was reused for different arguments",
            )),
            None => match registry.retired.get(&operation_id) {
                Some(retired) if retired.fingerprint == *fingerprint => {
                    Ok(Some(AcceptReplay::Complete(retired.result.clone())))
                }
                Some(_) => Err(pair_outcome_unknown(
                    "retired pair accept operation ID was reused for different arguments",
                )),
                None => Ok(None),
            },
        }
    }

    fn record_accept_completion(&self, operation_id: EphemeralOperationId, cell: &Arc<AcceptCell>) {
        let mut registry = mutex_lock(&self.accepts);
        if registry
            .cells
            .get(&operation_id)
            .is_some_and(|current| Arc::ptr_eq(current, cell))
        {
            registry.completed_order.push_back(operation_id);
        }
    }

    async fn execute_accept(
        self: Arc<Self>,
        pending: PendingAccept,
        deadline: Instant,
    ) -> AcceptOutcome {
        self.ensure_running()?;
        ensure_deadline(deadline, "pair accept operation deadline elapsed")?;
        let host_device_id = pending.fields.host_device_id();
        let remote_name = pending.remote_name.clone();
        let directory = self.directory.clone();
        let explicit_alias = pending.explicit_alias;
        let reservation = tokio::task::spawn_blocking(move || {
            directory.reserve_selected_alias(host_device_id, &remote_name, explicit_alias, deadline)
        })
        .await
        .map_err(|_| pair_outcome_unknown("pair alias reservation worker ended unexpectedly"))??;

        let routes = pending.fields.relay_hints().to_vec();
        let pair_attempt = self
            .run_controller_pair(&pending.fields, &pending.secret, routes.clone(), deadline)
            .await;
        let confirmation = self
            .transport
            .confirm_normal(host_device_id, routes, deadline)
            .await;
        let confirmation = resolve_normal_confirmation(host_device_id, pair_attempt, confirmation)?;

        let verified_route = match confirmation.verified_relay.clone() {
            Some(relay) => Some(RelayRouteCache {
                relay_hints: vec![relay],
                verified_at_unix: unix_now_i64().map_err(|_| {
                    pair_outcome_unknown(
                        "remote authorization was confirmed but route timestamping failed",
                    )
                })?,
            }),
            None => None,
        };
        let alias = reservation.alias().clone();
        let persisted_alias = alias.clone();
        let persisted_name = pending.remote_name.as_str().to_owned();
        let persist = self
            .store
            .run_blocking_until(deadline, move |store, deadline| {
                store.confirm_known_device(
                    host_device_id,
                    persisted_alias,
                    persisted_name,
                    verified_route,
                    deadline,
                )
            })
            .await;
        if persist.is_err() {
            return Err(pair_outcome_unknown(
                "remote authorization was confirmed but local device persistence did not complete",
            ));
        }

        Ok(PairAcceptResult {
            device_id: host_device_id,
            alias,
            remote_name: pending.remote_name,
            authorization_generation: confirmation.generation,
            verified_relay: confirmation.verified_relay,
        })
    }

    async fn run_controller_pair(
        &self,
        fields: &PairTicketFields,
        secret: &PairSecret,
        routes: Vec<RelayHint>,
        deadline: Instant,
    ) -> ControllerPairAttempt {
        let mut io = match self
            .transport
            .connect_pair(fields.host_device_id(), routes, deadline)
            .await
        {
            Ok(io) => io,
            Err(error) => return ControllerPairAttempt::failed(error, false),
        };
        if io.local() != self.identity.device_id()
            || io.remote() != fields.host_device_id()
            || io.local() == io.remote()
        {
            return ControllerPairAttempt::failed(
                invalid_pair_ticket("pairing TLS identity did not match the ticket"),
                false,
            );
        }

        let mut framing = match PairFraming::new(
            self.limits.max_pair_hello_frame_bytes,
            self.limits.max_pair_handshake_bytes,
            deadline,
        ) {
            Ok(framing) => framing,
            Err(error) => return ControllerPairAttempt::failed(error, false),
        };
        let controller_nonce = match self.manager.random_nonce_until(deadline) {
            Ok(nonce) => nonce,
            Err(error) => return ControllerPairAttempt::failed(error.into(), false),
        };
        let begin = match PairBegin::new(
            fields.offer_id(),
            self.identity.display_name(),
            controller_nonce,
            PAIR_PROTOCOL_VERSION,
        ) {
            Ok(begin) => begin,
            Err(_) => {
                return ControllerPairAttempt::failed(
                    invalid_pair_ticket("local pairing identity is invalid"),
                    false,
                );
            }
        };
        let mut begin_wire = v2::PairBegin::from(&begin);
        let begin_write = framing
            .write_message(&mut io, WireKind::PairBegin, &begin_wire, deadline)
            .await;
        begin_wire.offer_id.zeroize();
        begin_wire.controller_nonce.zeroize();
        if let Err(error) = begin_write {
            return ControllerPairAttempt::failed(error, false);
        }

        let challenge_wire: v2::PairChallenge = match framing
            .read_message(&mut io, WireKind::PairChallenge, deadline)
            .await
        {
            Ok(challenge) => challenge,
            Err(error) => return ControllerPairAttempt::failed(error, false),
        };
        let challenge = match pair_challenge_from_wire(challenge_wire) {
            Ok(challenge) => challenge,
            Err(error) => return ControllerPairAttempt::failed(error, false),
        };
        let transcript = match controller_transcript(
            fields,
            io.remote(),
            self.identity.device_id(),
            &begin,
            &challenge,
        ) {
            Ok(transcript) => transcript,
            Err(error) => return ControllerPairAttempt::failed(error.into(), false),
        };
        let offer_key = Zeroizing::new(fields.offer_key(secret));
        let controller_proof = Zeroizing::new(transcript.controller_proof(&offer_key));
        let mut proof_wire = v2::PairProof {
            controller_proof: controller_proof.to_vec(),
        };
        // A cancelled write_all may already have delivered a complete frame;
        // normal confirmation is therefore mandatory from this point onward.
        let proof_write = framing
            .write_message(&mut io, WireKind::PairProof, &proof_wire, deadline)
            .await;
        proof_wire.controller_proof.zeroize();
        if let Err(error) = proof_write {
            return ControllerPairAttempt::failed(error, true);
        }

        let mut accepted_wire: v2::PairAccepted = match framing
            .read_message(&mut io, WireKind::PairAccepted, deadline)
            .await
        {
            Ok(accepted) => accepted,
            Err(error) => return ControllerPairAttempt::failed(error, true),
        };
        let accepted = validate_pair_accepted(&mut accepted_wire, &transcript, &offer_key);
        let generation = match accepted {
            Ok(generation) => generation,
            Err(error) => return ControllerPairAttempt::failed(error, true),
        };
        if let Err(error) = framing.shutdown(&mut io, deadline).await {
            return ControllerPairAttempt {
                accepted_generation: Some(generation),
                repair_required: true,
                error: Some(error),
            };
        }
        ControllerPairAttempt {
            accepted_generation: Some(generation),
            repair_required: true,
            error: None,
        }
    }

    async fn run_host(
        &self,
        mut io: Box<dyn PairProtocolIo>,
        deadline: Instant,
    ) -> Result<(), DaemonError> {
        ensure_deadline(deadline, "inbound pairing handshake deadline elapsed")?;
        if io.local() != self.identity.device_id() || io.remote() == io.local() {
            return Err(DaemonError::new(
                DomainErrorKind::Unauthorized,
                "inbound pairing connection identity was rejected",
            ));
        }
        let controller_device_id = io.remote();
        let mut framing = PairFraming::new(
            self.limits.max_pair_hello_frame_bytes,
            self.limits.max_pair_handshake_bytes,
            deadline,
        )?;
        let first_frame_deadline = Instant::now()
            .checked_add(self.limits.first_frame_deadline)
            .unwrap_or(deadline)
            .min(deadline);

        let begin_wire: v2::PairBegin = framing
            .read_message(&mut io, WireKind::PairBegin, first_frame_deadline)
            .await?;
        let begin = PairBegin::try_from(begin_wire)
            .map_err(|_| PairingError::InvalidBinding.peer_error())?;
        let prepared = self
            .manager
            .prepare_challenge_until(controller_device_id, &begin, deadline)
            .map_err(PairingError::peer_error)?;
        let challenge_wire = v2::PairChallenge::from(prepared.challenge());
        framing
            .write_message(&mut io, WireKind::PairChallenge, &challenge_wire, deadline)
            .await?;

        let mut proof_wire: v2::PairProof = framing
            .read_message(&mut io, WireKind::PairProof, deadline)
            .await?;
        let proof = PairProof::from_slice(&proof_wire.controller_proof)
            .map_err(|_| PairingError::InvalidBinding.peer_error());
        proof_wire.controller_proof.zeroize();
        let proof = proof?;
        let consumption = self
            .manager
            .try_consume_until(prepared, &proof, deadline)
            .map_err(PairingError::peer_error)?;

        self.authorize_consumption(
            consumption,
            controller_device_id,
            &mut framing,
            &mut io,
            deadline,
        )
        .await
    }

    async fn authorize_consumption(
        &self,
        consumption: PairConsumption,
        controller_device_id: DeviceId,
        framing: &mut PairFraming,
        io: &mut Box<dyn PairProtocolIo>,
        deadline: Instant,
    ) -> Result<(), DaemonError> {
        let mut guard = match timeout_until(
            deadline,
            self.authorization.authorize_guard(controller_device_id),
        )
        .await
        {
            Ok(Ok(guard)) => guard,
            Ok(Err(_)) | Err(_) => {
                return Err(self.rollback_with_generic_rejection(consumption));
            }
        };
        let prior = guard.snapshot();
        let controller_name = consumption.controller_name().as_str().to_owned();
        let now_unix = match unix_now_i64() {
            Ok(now_unix) => now_unix,
            Err(_) => return Err(self.rollback_with_generic_rejection(consumption)),
        };
        let authorize = self
            .authorize_store(controller_device_id, controller_name, now_unix, deadline)
            .await;

        let generation = match authorize {
            Ok(generation) => generation,
            Err(error) if error.kind() == DomainErrorKind::OperationOutcomeUnknown => {
                let snapshot = self
                    .store
                    .run_blocking_until(deadline, move |store, deadline| {
                        store.authorization_snapshot(controller_device_id, deadline)
                    })
                    .await;
                match snapshot {
                    Ok(snapshot)
                        if snapshot.status == AuthorizationStatus::Authorized
                            && snapshot.generation > prior.generation =>
                    {
                        snapshot.generation
                    }
                    Ok(_) | Err(_) => {
                        // The authorize command may have committed. Dropping
                        // the permit intentionally leaves the offer Consuming.
                        drop(consumption);
                        return Err(generic_pair_rejection());
                    }
                }
            }
            Err(_) => return Err(self.rollback_with_generic_rejection(consumption)),
        };

        let durable = AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation,
        };
        let committed = match self
            .manager
            .commit(consumption, generation, &self.diagnostic_version)
        {
            Ok(committed) => committed,
            Err(error) => {
                // SQLite is already authoritative. Publish its truth for the
                // normal-confirmation repair oracle and leave the offer closed.
                let _ = guard.publish(durable);
                return Err(error.peer_error());
            }
        };
        guard
            .publish(durable)
            .map_err(|_| generic_pair_rejection())?;

        tracing::info!(
            component = "pairing",
            operation = "inbound_authorized",
            generation = generation.get(),
            "Inbound authorization committed"
        );
        let accepted = committed
            .pair_accepted()
            .map_err(PairingError::peer_error)?;
        let mut accepted_wire = v2::PairAccepted::from(&accepted);
        let write = framing
            .write_message(&mut *io, WireKind::PairAccepted, &accepted_wire, deadline)
            .await;
        accepted_wire.host_confirmation_proof.zeroize();
        write?;
        framing.shutdown(&mut *io, deadline).await
    }

    fn rollback_with_generic_rejection(&self, consumption: PairConsumption) -> DaemonError {
        let _ = self.manager.rollback(consumption);
        generic_pair_rejection()
    }

    async fn authorize_store(
        &self,
        controller_device_id: DeviceId,
        controller_name: String,
        now_unix: i64,
        deadline: Instant,
    ) -> Result<AuthGeneration, DaemonError> {
        #[cfg(test)]
        let injected = mutex_lock(&self.authorize_faults).pop_front();
        #[cfg(test)]
        if injected == Some(AuthorizeFault::OutcomeUnknownBeforeCommit) {
            return Err(DaemonError::new(
                DomainErrorKind::OperationOutcomeUnknown,
                "injected pre-commit outcome ambiguity",
            ));
        }

        let result = self
            .store
            .run_blocking_until(deadline, move |store, deadline| {
                store.authorize(controller_device_id, controller_name, now_unix, deadline)
            })
            .await;

        #[cfg(test)]
        if injected == Some(AuthorizeFault::OutcomeUnknownAfterCommit) && result.is_ok() {
            return Err(DaemonError::new(
                DomainErrorKind::OperationOutcomeUnknown,
                "injected post-commit response loss",
            ));
        }
        result
    }

    #[cfg(test)]
    fn inject_authorize_fault(&self, fault: AuthorizeFault) {
        mutex_lock(&self.authorize_faults).push_back(fault);
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthorizeFault {
    OutcomeUnknownBeforeCommit,
    OutcomeUnknownAfterCommit,
}

impl PairConnectionHandler for PairingService {
    fn handle_pair_connection(
        &self,
        connection: PairConnection,
        deadline: Instant,
    ) -> PairConnectionHandlerFuture {
        let service = self.clone();
        Box::pin(async move { service.accept_pair_connection(connection, deadline).await })
    }
}

trait NetworkStatusSource: Send + Sync {
    fn snapshot(&self) -> NetworkObservation;
}

impl NetworkStatusSource for NetworkObserver {
    fn snapshot(&self) -> NetworkObservation {
        NetworkObserver::snapshot(self)
    }
}

trait PairProtocolIo: AsyncRead + AsyncWrite + Unpin + Send {
    fn local(&self) -> DeviceId;
    fn remote(&self) -> DeviceId;
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConfirmedAuthorization {
    remote: DeviceId,
    generation: AuthGeneration,
    verified_relay: Option<RelayHint>,
}

struct PendingAccept {
    fields: PairTicketFields,
    secret: PairSecret,
    remote_name: DeviceDisplayName,
    explicit_alias: Option<DeviceAlias>,
}

struct ControllerPairAttempt {
    accepted_generation: Option<AuthGeneration>,
    repair_required: bool,
    error: Option<DaemonError>,
}

impl ControllerPairAttempt {
    fn failed(error: DaemonError, repair_required: bool) -> Self {
        Self {
            accepted_generation: None,
            repair_required,
            error: Some(error),
        }
    }
}

fn resolve_normal_confirmation(
    expected_remote: DeviceId,
    pair_attempt: ControllerPairAttempt,
    confirmation: Result<ConfirmedAuthorization, DaemonError>,
) -> Result<ConfirmedAuthorization, DaemonError> {
    let confirmation = match confirmation {
        Ok(confirmation)
            if confirmation.remote == expected_remote
                && confirmation.generation != AuthGeneration::ZERO =>
        {
            confirmation
        }
        Ok(_) => {
            return Err(pair_outcome_unknown(
                "normal confirmation returned inconsistent authorization",
            ));
        }
        Err(normal_error) => {
            if pair_attempt.repair_required {
                return Err(pair_outcome_unknown(
                    "remote pairing outcome could not be confirmed",
                ));
            }
            return Err(pair_attempt.error.unwrap_or(normal_error));
        }
    };
    if pair_attempt
        .accepted_generation
        .is_some_and(|accepted| confirmation.generation < accepted)
    {
        return Err(pair_outcome_unknown(
            "normal confirmation generation predates PairAccepted",
        ));
    }
    Ok(confirmation)
}

trait PairTransport: Send + Sync {
    fn connect_pair<'a>(
        &'a self,
        remote: DeviceId,
        routes: Vec<RelayHint>,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Box<dyn PairProtocolIo>, DaemonError>>;

    fn confirm_normal<'a>(
        &'a self,
        remote: DeviceId,
        routes: Vec<RelayHint>,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<ConfirmedAuthorization, DaemonError>>;
}

struct BrokerPairTransport {
    broker: ConnectionBroker,
}

struct BrokerPairIo {
    _connection: PairConnection,
    send: SendStream,
    recv: RecvStream,
    local: DeviceId,
    remote: DeviceId,
}

impl BrokerPairIo {
    async fn open(connection: PairConnection, deadline: Instant) -> Result<Self, DaemonError> {
        let local = connection.local();
        let remote = connection.remote();
        let (send, recv) = connection.open_bi(deadline).await?;
        Ok(Self {
            _connection: connection,
            send,
            recv,
            local,
            remote,
        })
    }

    async fn accept(connection: PairConnection, deadline: Instant) -> Result<Self, DaemonError> {
        let local = connection.local();
        let remote = connection.remote();
        let (send, recv) = connection.accept_bi(deadline).await?;
        Ok(Self {
            _connection: connection,
            send,
            recv,
            local,
            remote,
        })
    }
}

impl PairProtocolIo for BrokerPairIo {
    fn local(&self) -> DeviceId {
        self.local
    }

    fn remote(&self) -> DeviceId {
        self.remote
    }
}

impl AsyncRead for BrokerPairIo {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        AsyncRead::poll_read(Pin::new(&mut self.get_mut().recv), context, buffer)
    }
}

impl AsyncWrite for BrokerPairIo {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.get_mut().send), context, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.get_mut().send), context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.get_mut().send), context)
    }
}

impl PairTransport for BrokerPairTransport {
    fn connect_pair<'a>(
        &'a self,
        remote: DeviceId,
        routes: Vec<RelayHint>,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Box<dyn PairProtocolIo>, DaemonError>> {
        Box::pin(async move {
            ensure_deadline(deadline, "pairing connection deadline elapsed")?;
            let connection = self
                .broker
                .connect_pair_transient(remote, routes, deadline)
                .await?;
            let io = BrokerPairIo::open(connection, deadline).await?;
            Ok(Box::new(io) as Box<dyn PairProtocolIo>)
        })
    }

    fn confirm_normal<'a>(
        &'a self,
        remote: DeviceId,
        routes: Vec<RelayHint>,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<ConfirmedAuthorization, DaemonError>> {
        Box::pin(async move {
            ensure_deadline(deadline, "normal confirmation deadline elapsed")?;
            let demand = self.broker.demand_transient(remote, routes).await?;
            let confirmation = demand.confirm_authorization(deadline).await?;
            if confirmation.remote() != remote {
                return Err(DaemonError::new(
                    DomainErrorKind::Unauthorized,
                    "normal confirmation identity did not match the pairing ticket",
                ));
            }
            Ok(ConfirmedAuthorization {
                remote: confirmation.remote(),
                generation: confirmation.generation(),
                verified_relay: confirmation.verified_relay().cloned(),
            })
        })
    }
}

#[derive(Default)]
struct AcceptRegistry {
    cells: BTreeMap<EphemeralOperationId, Arc<AcceptCell>>,
    completed_order: VecDeque<EphemeralOperationId>,
    retired: BTreeMap<EphemeralOperationId, RetiredAccept>,
    retired_order: VecDeque<EphemeralOperationId>,
}

struct RetiredAccept {
    fingerprint: PairFingerprint,
    result: AcceptOutcome,
}

enum AcceptReplay {
    Live(Arc<AcceptCell>),
    Complete(AcceptOutcome),
}

enum AcceptAdmission {
    Created(Arc<AcceptCell>),
    Live(Arc<AcceptCell>),
    Complete(AcceptOutcome),
}

struct AcceptCell {
    fingerprint: PairFingerprint,
    result: Mutex<Option<AcceptOutcome>>,
    notify: Notify,
}

impl AcceptCell {
    fn new(fingerprint: PairFingerprint) -> Self {
        Self {
            fingerprint,
            result: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    fn complete(&self, result: AcceptOutcome) {
        *mutex_lock(&self.result) = Some(result);
        self.notify.notify_waiters();
    }

    async fn wait_until(
        &self,
        deadline: Instant,
        mut shutdown: watch::Receiver<bool>,
    ) -> AcceptOutcome {
        loop {
            let notified = self.notify.notified();
            if let Some(result) = mutex_lock(&self.result).clone() {
                return result;
            }
            if *shutdown.borrow() {
                return Err(cancelled("pairing service is stopping"));
            }
            if Instant::now() >= deadline {
                return Err(deadline_exceeded("local pair accept deadline elapsed"));
            }
            tokio::select! {
                _ = notified => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Err(cancelled("pairing service is stopping"));
                    }
                }
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                    return Err(deadline_exceeded("local pair accept deadline elapsed"));
                }
            }
        }
    }
}

fn retire_oldest_accept(registry: &mut AcceptRegistry, maximum_retired: usize) -> bool {
    while let Some(operation_id) = registry.completed_order.pop_front() {
        let Some(cell) = registry.cells.get(&operation_id) else {
            continue;
        };
        let Some(result) = mutex_lock(&cell.result).clone() else {
            continue;
        };
        let fingerprint = cell.fingerprint.clone();
        registry.cells.remove(&operation_id);
        registry.retired.insert(
            operation_id,
            RetiredAccept {
                fingerprint,
                result,
            },
        );
        registry.retired_order.push_back(operation_id);
        while registry.retired.len() > maximum_retired {
            let Some(expired) = registry.retired_order.pop_front() else {
                break;
            };
            registry.retired.remove(&expired);
        }
        return true;
    }
    false
}

fn mutex_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn deadline_exceeded(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::DeadlineExceeded, detail)
}

fn cancelled(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::Cancelled, detail)
}

fn generic_pair_rejection() -> DaemonError {
    PairingError::InvalidBinding.peer_error()
}

fn invalid_pair_ticket(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::PairTicketInvalid, detail)
}

fn pair_challenge_from_wire(mut wire: v2::PairChallenge) -> Result<PairChallenge, DaemonError> {
    let nonce = PairNonce::from_bytes(&wire.host_nonce)
        .map_err(|_| invalid_pair_ticket("pairing challenge nonce is invalid"));
    wire.host_nonce.zeroize();
    PairChallenge::new(nonce?, wire.selected_version, wire.ticket_expiry_unix)
        .map_err(|_| invalid_pair_ticket("pairing challenge fields are invalid"))
}

fn validate_pair_accepted(
    wire: &mut v2::PairAccepted,
    transcript: &zterm_core::PairTranscript,
    offer_key: &[u8; 32],
) -> Result<AuthGeneration, DaemonError> {
    let generation = AuthGeneration::new(wire.authorization_generation)
        .filter(|generation| *generation != AuthGeneration::ZERO)
        .ok_or_else(|| invalid_pair_ticket("PairAccepted generation is invalid"));
    let proof = <[u8; 32]>::try_from(wire.host_confirmation_proof.as_slice())
        .map(Zeroizing::new)
        .map_err(|_| invalid_pair_ticket("PairAccepted confirmation proof is invalid"));
    wire.host_confirmation_proof.zeroize();
    DeviceDisplayName::new(std::mem::take(&mut wire.host_diagnostic_version))
        .map_err(|_| invalid_pair_ticket("PairAccepted diagnostic version is invalid"))?;
    let generation = generation?;
    let proof = proof?;
    if !transcript.verify_host_confirmation(offer_key, generation.get(), &proof) {
        return Err(invalid_pair_ticket(
            "PairAccepted confirmation proof was rejected",
        ));
    }
    Ok(generation)
}

fn unix_now_u64() -> Result<u64, DaemonError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            DaemonError::new(
                DomainErrorKind::TransportUnavailable,
                "pairing clock is unavailable",
            )
        })
        .map(|duration| duration.as_secs())
}

fn unix_now_i64() -> Result<i64, DaemonError> {
    let seconds = unix_now_u64()?;
    i64::try_from(seconds).map_err(|_| {
        DaemonError::new(
            DomainErrorKind::TransportUnavailable,
            "pairing clock is outside the supported range",
        )
    })
}

fn pair_outcome_unknown(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::PairOutcomeUnknown, detail)
}

fn ensure_deadline(deadline: Instant, detail: &'static str) -> Result<(), DaemonError> {
    if Instant::now() >= deadline {
        Err(deadline_exceeded(detail))
    } else {
        Ok(())
    }
}

async fn timeout_until<F>(deadline: Instant, future: F) -> Result<F::Output, DaemonError>
where
    F: Future,
{
    ensure_deadline(deadline, "pairing transport deadline elapsed")?;
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future)
        .await
        .map_err(|_| deadline_exceeded("pairing transport deadline elapsed"))
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use zterm_core::{Capabilities, PAIR_TICKET_FORMAT_VERSION, PairOfferId};
    use zterm_platform::user_state::UserPaths;

    use super::*;
    use crate::pairing::{
        PairOfferState, PairingClock, PairingClockError, PairingEntropy, PairingEntropyError,
        PairingNow,
    };
    use crate::store::{StateStore, StoreActor};

    const CONTROLLER_NONCE_BYTE: u8 = 0x5a;
    const HOST_NONCE_BYTE: u8 = 0x6b;

    struct FixedEntropy;

    impl PairingEntropy for FixedEntropy {
        fn fill(&self, destination: &mut [u8]) -> Result<(), PairingEntropyError> {
            destination.fill(CONTROLLER_NONCE_BYTE);
            Ok(())
        }
    }

    struct LiveClock;

    impl PairingClock for LiveClock {
        fn now(&self) -> Result<PairingNow, PairingClockError> {
            Ok(PairingNow::new(
                unix_now_u64().map_err(|_| PairingClockError)?,
                Instant::now(),
            ))
        }
    }

    struct StaticNetwork {
        observation: Mutex<NetworkObservation>,
    }

    impl StaticNetwork {
        fn new(device_id: DeviceId) -> Self {
            Self {
                observation: Mutex::new(NetworkObservation::disabled(device_id)),
            }
        }

        fn set(&self, observation: NetworkObservation) {
            *mutex_lock(&self.observation) = observation;
        }
    }

    impl NetworkStatusSource for StaticNetwork {
        fn snapshot(&self) -> NetworkObservation {
            mutex_lock(&self.observation).clone()
        }
    }

    struct MemoryPairIo {
        input: Zeroizing<Vec<u8>>,
        input_position: usize,
        output: Arc<Mutex<Zeroizing<Vec<u8>>>>,
        write_count: usize,
        fail_on_write: Option<usize>,
        local: DeviceId,
        remote: DeviceId,
    }

    impl MemoryPairIo {
        fn new(input: Vec<u8>, local: DeviceId, remote: DeviceId) -> Self {
            Self {
                input: Zeroizing::new(input),
                input_position: 0,
                output: Arc::new(Mutex::new(Zeroizing::new(Vec::new()))),
                write_count: 0,
                fail_on_write: None,
                local,
                remote,
            }
        }

        fn failing_on_write(mut self, write_number: usize) -> Self {
            self.fail_on_write = Some(write_number);
            self
        }
    }

    impl PairProtocolIo for MemoryPairIo {
        fn local(&self) -> DeviceId {
            self.local
        }

        fn remote(&self) -> DeviceId {
            self.remote
        }
    }

    impl AsyncRead for MemoryPairIo {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let position = self.input_position;
            let count = {
                let available = self.input.get(position..).unwrap_or_default();
                let count = available.len().min(buffer.remaining());
                buffer.put_slice(&available[..count]);
                count
            };
            self.input_position = position.saturating_add(count);
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for MemoryPairIo {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.write_count = self.write_count.saturating_add(1);
            if self.fail_on_write == Some(self.write_count) {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "injected in-memory pair write failure",
                )));
            }
            mutex_lock(&self.output).extend_from_slice(buffer);
            Poll::Ready(Ok(buffer.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    struct FakeTransport {
        connections: Mutex<VecDeque<Result<MemoryPairIo, DaemonError>>>,
        confirmations: Mutex<VecDeque<Result<ConfirmedAuthorization, DaemonError>>>,
        connect_calls: AtomicUsize,
        confirmation_calls: AtomicUsize,
    }

    impl FakeTransport {
        fn new(
            connection: Result<MemoryPairIo, DaemonError>,
            confirmation: Result<ConfirmedAuthorization, DaemonError>,
        ) -> Self {
            Self {
                connections: Mutex::new(VecDeque::from([connection])),
                confirmations: Mutex::new(VecDeque::from([confirmation])),
                connect_calls: AtomicUsize::new(0),
                confirmation_calls: AtomicUsize::new(0),
            }
        }

        fn empty() -> Self {
            Self {
                connections: Mutex::new(VecDeque::new()),
                confirmations: Mutex::new(VecDeque::new()),
                connect_calls: AtomicUsize::new(0),
                confirmation_calls: AtomicUsize::new(0),
            }
        }
    }

    impl PairTransport for FakeTransport {
        fn connect_pair<'a>(
            &'a self,
            _remote: DeviceId,
            _routes: Vec<RelayHint>,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<Box<dyn PairProtocolIo>, DaemonError>> {
            self.connect_calls.fetch_add(1, Ordering::SeqCst);
            let result = mutex_lock(&self.connections)
                .pop_front()
                .unwrap_or_else(|| Err(cancelled("fake pair connection was already consumed")))
                .map(|io| Box::new(io) as Box<dyn PairProtocolIo>);
            Box::pin(async move { result })
        }

        fn confirm_normal<'a>(
            &'a self,
            _remote: DeviceId,
            _routes: Vec<RelayHint>,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<ConfirmedAuthorization, DaemonError>> {
            self.confirmation_calls.fetch_add(1, Ordering::SeqCst);
            let result = mutex_lock(&self.confirmations)
                .pop_front()
                .unwrap_or_else(|| Err(cancelled("fake normal confirmation was already consumed")));
            Box::pin(async move { result })
        }
    }

    struct BlockingTransport {
        connect_calls: AtomicUsize,
        confirmation_calls: AtomicUsize,
        connect_started: Notify,
        release_connect: Notify,
    }

    impl BlockingTransport {
        fn new() -> Self {
            Self {
                connect_calls: AtomicUsize::new(0),
                confirmation_calls: AtomicUsize::new(0),
                connect_started: Notify::new(),
                release_connect: Notify::new(),
            }
        }
    }

    impl PairTransport for BlockingTransport {
        fn connect_pair<'a>(
            &'a self,
            _remote: DeviceId,
            _routes: Vec<RelayHint>,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<Box<dyn PairProtocolIo>, DaemonError>> {
            Box::pin(async move {
                self.connect_calls.fetch_add(1, Ordering::SeqCst);
                self.connect_started.notify_one();
                self.release_connect.notified().await;
                Err(address_unavailable())
            })
        }

        fn confirm_normal<'a>(
            &'a self,
            _remote: DeviceId,
            _routes: Vec<RelayHint>,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<ConfirmedAuthorization, DaemonError>> {
            self.confirmation_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(address_unavailable()) })
        }
    }

    struct Harness {
        service: PairingService,
        store: StoreHandle,
        network: Arc<StaticNetwork>,
        authorization: AuthorizationRegistry,
        database: PathBuf,
        _actor: StoreActor,
        _temporary: tempfile::TempDir,
    }

    fn harness(transport: Arc<dyn PairTransport>) -> Harness {
        let local = device_id(0x11);
        let temporary = tempfile::tempdir().expect("temporary state root");
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
        let database = paths.database().to_path_buf();
        let actor =
            StoreActor::start(StateStore::open(&paths).expect("state store")).expect("store actor");
        let store = actor.handle();
        let authorization = AuthorizationRegistry::new();
        let directory = DeviceDirectory::new(store.clone());
        let limits = TransportLimits::default();
        let manager = PairingManager::with_dependencies(
            local,
            limits,
            Arc::new(LiveClock),
            Arc::new(FixedEntropy),
        )
        .expect("pairing manager");
        let identity = ConnectionIdentity::new(
            local,
            "controller",
            "test-build",
            "test-platform",
            Capabilities::from_bits_retain(0),
        )
        .expect("connection identity");
        let network = Arc::new(StaticNetwork::new(local));
        let service = PairingService::with_dependencies(
            manager,
            store.clone(),
            authorization.clone(),
            directory,
            transport,
            network.clone(),
            identity,
            limits,
        )
        .expect("pairing service");
        Harness {
            service,
            store,
            network,
            authorization,
            database,
            _actor: actor,
            _temporary: temporary,
        }
    }

    struct TicketFixture {
        text: Zeroizing<String>,
        fields: PairTicketFields,
    }

    fn ticket(host: DeviceId) -> TicketFixture {
        let expiry = unix_now_u64().expect("clock") + DEFAULT_PAIR_TTL_SECONDS;
        let fields = PairTicketFields::new(
            PAIR_TICKET_FORMAT_VERSION,
            host,
            "paired-host",
            vec![RelayHint::new("https://relay.example.test").expect("relay")],
            PairOfferId::from_array([0x33; 16]),
            expiry,
        )
        .expect("ticket fields");
        let secret = PairSecret::from_bytes([0x44; 32]);
        let text = Zeroizing::new(zterm_proto::encode_pair_ticket(&fields, &secret));
        TicketFixture { text, fields }
    }

    fn device_id(seed: u8) -> DeviceId {
        let secret = iroh::SecretKey::from_bytes(&[seed; 32]);
        DeviceId::from_array(*secret.public().as_bytes())
    }

    fn encoded<Message: prost::Message>(kind: WireKind, message: &Message) -> Vec<u8> {
        zterm_proto::encode_message(kind, 0, 0, message).expect("bounded pair frame")
    }

    fn valid_challenge(ticket: &TicketFixture) -> Vec<u8> {
        let challenge = PairChallenge::new(
            PairNonce::from_array([HOST_NONCE_BYTE; 32]),
            PAIR_PROTOCOL_VERSION,
            ticket.fields.expires_at_unix(),
        )
        .expect("challenge");
        encoded(
            WireKind::PairChallenge,
            &v2::PairChallenge::from(&challenge),
        )
    }

    fn accept_input(ticket: &TicketFixture, operation_byte: u8) -> LocalPairAcceptInput {
        let alias = DeviceAlias::new("paired-host-alias").expect("alias");
        LocalPairAcceptInput::new(
            EphemeralOperationId::from_array([operation_byte; 16]),
            PairFingerprint::for_accept(ticket.text.as_bytes(), Some(&alias)),
            ticket.text.as_str().to_owned(),
            Some(alias),
        )
    }

    struct HostAttempt {
        offer_id: PairOfferId,
        io: MemoryPairIo,
    }

    fn host_attempt(
        harness: &Harness,
        controller: DeviceId,
        operation_byte: u8,
        valid_proof: bool,
    ) -> HostAttempt {
        let ttl = DEFAULT_PAIR_TTL_SECONDS;
        let request = PairOfferRequest::new(
            EphemeralOperationId::from_array([operation_byte; 16]),
            PairFingerprint::for_create(ttl),
            DeviceDisplayName::new("pair-host").expect("host name"),
            vec![RelayHint::new("https://relay.example.test").expect("relay")],
            ttl,
        )
        .expect("host offer request");
        let created = harness
            .service
            .inner
            .manager
            .create_offer(request)
            .expect("host offer");
        let offer_id = created.fields().offer_id();
        let (fields, secret) =
            zterm_proto::decode_pair_ticket(created.ticket().expose()).expect("host ticket");
        let begin = PairBegin::new(
            offer_id,
            "remote-controller",
            PairNonce::from_array([0x79; 32]),
            PAIR_PROTOCOL_VERSION,
        )
        .expect("PairBegin");
        let challenge = PairChallenge::new(
            PairNonce::from_array([CONTROLLER_NONCE_BYTE; 32]),
            PAIR_PROTOCOL_VERSION,
            fields.expires_at_unix(),
        )
        .expect("host challenge");
        let transcript = controller_transcript(
            &fields,
            harness.service.inner.identity.device_id(),
            controller,
            &begin,
            &challenge,
        )
        .expect("host transcript");
        let offer_key = Zeroizing::new(fields.offer_key(&secret));
        let proof = if valid_proof {
            Zeroizing::new(transcript.controller_proof(&offer_key))
        } else {
            Zeroizing::new([0xee; 32])
        };

        let mut begin_wire = v2::PairBegin::from(&begin);
        let mut input = encoded(WireKind::PairBegin, &begin_wire);
        begin_wire.offer_id.zeroize();
        begin_wire.controller_nonce.zeroize();
        let mut proof_wire = v2::PairProof {
            controller_proof: proof.to_vec(),
        };
        let proof_frame = Zeroizing::new(encoded(WireKind::PairProof, &proof_wire));
        proof_wire.controller_proof.zeroize();
        input.extend_from_slice(&proof_frame);

        HostAttempt {
            offer_id,
            io: MemoryPairIo::new(
                input,
                harness.service.inner.identity.device_id(),
                controller,
            ),
        }
    }

    fn assert_offer_state(harness: &Harness, offer_id: PairOfferId, expected: PairOfferState) {
        assert_eq!(
            harness.service.inner.manager.offer_state(offer_id),
            Ok(expected)
        );
    }

    fn durable_authorization(harness: &Harness, device_id: DeviceId) -> AuthorizationSnapshot {
        harness
            .store
            .authorization_snapshot(device_id, Instant::now() + Duration::from_secs(2))
            .expect("durable authorization snapshot")
    }

    fn address_unavailable() -> DaemonError {
        DaemonError::new(
            DomainErrorKind::AddressUnavailable,
            "fake normal route unavailable",
        )
    }

    #[tokio::test]
    async fn pre_proof_failure_preserves_typed_error_when_normal_confirmation_fails() {
        let local = device_id(0x11);
        let host = device_id(0x22);
        let ticket = ticket(host);
        let invalid = v2::PairChallenge {
            host_nonce: vec![0; 31],
            selected_version: PAIR_PROTOCOL_VERSION,
            ticket_expiry_unix: ticket.fields.expires_at_unix(),
        };
        let transport = Arc::new(FakeTransport::new(
            Ok(MemoryPairIo::new(
                encoded(WireKind::PairChallenge, &invalid),
                local,
                host,
            )),
            Err(address_unavailable()),
        ));
        let harness = harness(transport);

        let error = harness
            .service
            .accept_until(
                accept_input(&ticket, 0x51),
                Instant::now() + Duration::from_secs(2),
            )
            .await
            .expect_err("pre-proof challenge failure is typed");
        assert_eq!(error.kind(), DomainErrorKind::PairTicketInvalid);
    }

    #[tokio::test]
    async fn post_proof_failure_and_normal_failure_is_outcome_unknown() {
        let local = device_id(0x11);
        let host = device_id(0x22);
        let ticket = ticket(host);
        let transport = Arc::new(FakeTransport::new(
            Ok(MemoryPairIo::new(valid_challenge(&ticket), local, host)),
            Err(address_unavailable()),
        ));
        let harness = harness(transport);

        let error = harness
            .service
            .accept_until(
                accept_input(&ticket, 0x52),
                Instant::now() + Duration::from_secs(2),
            )
            .await
            .expect_err("PairAccepted loss without normal proof is ambiguous");
        assert_eq!(error.kind(), DomainErrorKind::PairOutcomeUnknown);
    }

    #[tokio::test]
    async fn pair_accepted_drop_normal_confirmation_repairs_one_way_known_device() {
        let local = device_id(0x11);
        let host = device_id(0x22);
        let ticket = ticket(host);
        let verified_relay = RelayHint::new("https://verified.example.test").expect("relay");
        let generation = AuthGeneration::new(7).expect("generation");
        let transport = Arc::new(FakeTransport::new(
            Ok(MemoryPairIo::new(valid_challenge(&ticket), local, host)),
            Ok(ConfirmedAuthorization {
                remote: host,
                generation,
                verified_relay: Some(verified_relay.clone()),
            }),
        ));
        let harness = harness(transport);

        let accepted = harness
            .service
            .accept_until(
                accept_input(&ticket, 0x53),
                Instant::now() + Duration::from_secs(2),
            )
            .await
            .expect("normal confirmation repairs PairAccepted loss");
        assert_eq!(accepted.device_id(), host);
        assert_eq!(accepted.authorization_generation(), generation);
        assert_eq!(accepted.verified_relay(), Some(&verified_relay));

        let deadline = Instant::now() + Duration::from_secs(2);
        let known = harness
            .store
            .known_device(host, deadline)
            .expect("known-device read")
            .expect("known-device row");
        assert_eq!(known.local_alias, *accepted.alias());
        assert_eq!(known.remote_name, *accepted.remote_name());
        assert_eq!(
            known.route_cache.expect("verified route").relay_hints,
            vec![verified_relay]
        );
        assert_eq!(
            harness
                .store
                .authorization_snapshot(host, deadline)
                .expect("controller-side auth snapshot"),
            AuthorizationSnapshot::none()
        );
    }

    #[tokio::test]
    async fn host_invalid_proof_leaves_offer_ready_and_store_untouched() {
        let transport = Arc::new(FakeTransport::empty());
        let harness = harness(transport);
        let controller = device_id(0x71);
        let attempt = host_attempt(&harness, controller, 0x81, false);

        let error = harness
            .service
            .inner
            .run_host(
                Box::new(attempt.io),
                Instant::now() + Duration::from_secs(2),
            )
            .await
            .expect_err("invalid proof is generically rejected");
        assert_eq!(error.kind(), DomainErrorKind::PairTicketInvalid);
        assert_offer_state(&harness, attempt.offer_id, PairOfferState::Ready);
        assert_eq!(
            durable_authorization(&harness, controller),
            AuthorizationSnapshot::none()
        );
        assert_eq!(
            harness
                .authorization
                .snapshot(controller)
                .expect("registry snapshot"),
            AuthorizationSnapshot::none()
        );
    }

    #[tokio::test]
    async fn host_generation_exhaustion_rolls_offer_back_to_ready() {
        let transport = Arc::new(FakeTransport::empty());
        let harness = harness(transport);
        let controller = device_id(0x72);
        let deadline = Instant::now() + Duration::from_secs(2);
        harness
            .store
            .authorize(controller, "remote-controller", 1, deadline)
            .expect("seed authorization");
        rusqlite::Connection::open(&harness.database)
            .expect("fixture connection")
            .execute(
                "UPDATE device_auth SET generation=?1 WHERE endpoint_id=?2",
                rusqlite::params![i64::MAX, controller.as_bytes().as_slice()],
            )
            .expect("seed maximum generation");
        let attempt = host_attempt(&harness, controller, 0x82, true);

        let error = harness
            .service
            .inner
            .run_host(Box::new(attempt.io), deadline)
            .await
            .expect_err("generation exhaustion is rejected");
        assert_eq!(error.kind(), DomainErrorKind::PairTicketInvalid);
        assert_offer_state(&harness, attempt.offer_id, PairOfferState::Ready);
        assert_eq!(
            durable_authorization(&harness, controller),
            AuthorizationSnapshot {
                status: AuthorizationStatus::Authorized,
                generation: AuthGeneration::new(AuthGeneration::SQLITE_MAX)
                    .expect("maximum generation"),
            }
        );
        assert_eq!(
            harness
                .authorization
                .snapshot(controller)
                .expect("registry snapshot"),
            AuthorizationSnapshot::none(),
            "an exact store failure must not publish registry authorization"
        );
    }

    #[tokio::test]
    async fn host_started_unknown_reconciles_advanced_durable_generation() {
        let transport = Arc::new(FakeTransport::empty());
        let harness = harness(transport);
        let controller = device_id(0x73);
        harness
            .service
            .inner
            .inject_authorize_fault(AuthorizeFault::OutcomeUnknownAfterCommit);
        let attempt = host_attempt(&harness, controller, 0x83, true);

        harness
            .service
            .inner
            .run_host(
                Box::new(attempt.io),
                Instant::now() + Duration::from_secs(2),
            )
            .await
            .expect("advanced durable generation reconciles ambiguity");
        let expected = AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: AuthGeneration::new(1).expect("first generation"),
        };
        assert_offer_state(&harness, attempt.offer_id, PairOfferState::Consumed);
        assert_eq!(durable_authorization(&harness, controller), expected);
        assert_eq!(
            harness
                .authorization
                .snapshot(controller)
                .expect("registry snapshot"),
            expected
        );
    }

    #[tokio::test]
    async fn host_started_unknown_without_durable_proof_stays_consuming() {
        let transport = Arc::new(FakeTransport::empty());
        let harness = harness(transport);
        let controller = device_id(0x74);
        harness
            .service
            .inner
            .inject_authorize_fault(AuthorizeFault::OutcomeUnknownBeforeCommit);
        let attempt = host_attempt(&harness, controller, 0x84, true);

        let error = harness
            .service
            .inner
            .run_host(
                Box::new(attempt.io),
                Instant::now() + Duration::from_secs(2),
            )
            .await
            .expect_err("unprovable ambiguity fails closed");
        assert_eq!(error.kind(), DomainErrorKind::PairTicketInvalid);
        assert_offer_state(&harness, attempt.offer_id, PairOfferState::Consuming);
        assert_eq!(
            durable_authorization(&harness, controller),
            AuthorizationSnapshot::none()
        );
        assert_eq!(
            harness
                .authorization
                .snapshot(controller)
                .expect("registry snapshot"),
            AuthorizationSnapshot::none()
        );
    }

    #[tokio::test]
    async fn host_pair_accepted_write_drop_keeps_durable_and_published_authorization() {
        let transport = Arc::new(FakeTransport::empty());
        let harness = harness(transport);
        let controller = device_id(0x75);
        let mut attempt = host_attempt(&harness, controller, 0x85, true);
        attempt.io = attempt.io.failing_on_write(2);

        harness
            .service
            .inner
            .run_host(
                Box::new(attempt.io),
                Instant::now() + Duration::from_secs(2),
            )
            .await
            .expect_err("PairAccepted write is injected to fail");
        let expected = AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: AuthGeneration::new(1).expect("first generation"),
        };
        assert_offer_state(&harness, attempt.offer_id, PairOfferState::Consumed);
        assert_eq!(durable_authorization(&harness, controller), expected);
        assert_eq!(
            harness
                .authorization
                .snapshot(controller)
                .expect("registry snapshot"),
            expected,
            "authorization must publish before PairAccepted is attempted"
        );
    }

    #[tokio::test]
    async fn explicit_alias_conflict_is_rejected_before_pair_transport() {
        let transport = Arc::new(FakeTransport::empty());
        let harness = harness(transport.clone());
        let alias = DeviceAlias::new("paired-host-alias").expect("alias");
        harness
            .store
            .confirm_known_device(
                device_id(0x31),
                alias,
                "existing-device",
                None,
                Instant::now() + Duration::from_secs(2),
            )
            .expect("seed conflicting alias");
        let ticket = ticket(device_id(0x32));

        let error = harness
            .service
            .accept_until(
                accept_input(&ticket, 0x91),
                Instant::now() + Duration::from_secs(2),
            )
            .await
            .expect_err("explicit alias conflict");
        assert_eq!(error.kind(), DomainErrorKind::DeviceAliasConflict);
        assert_eq!(transport.connect_calls.load(Ordering::SeqCst), 0);
        assert_eq!(transport.confirmation_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn concurrent_same_accept_operation_joins_one_transport_task() {
        let transport = Arc::new(BlockingTransport::new());
        let harness = harness(transport.clone());
        let ticket = ticket(device_id(0x33));
        let first_input = accept_input(&ticket, 0x92);
        let second_input = accept_input(&ticket, 0x92);
        let first_service = harness.service.clone();
        let deadline = Instant::now() + Duration::from_secs(2);
        let first =
            tokio::spawn(async move { first_service.accept_until(first_input, deadline).await });
        tokio::time::timeout(Duration::from_secs(1), transport.connect_started.notified())
            .await
            .expect("first operation reaches fake transport");

        let second_service = harness.service.clone();
        let second =
            tokio::spawn(async move { second_service.accept_until(second_input, deadline).await });
        tokio::task::yield_now().await;
        transport.release_connect.notify_waiters();

        let first_error = first
            .await
            .expect("first waiter task")
            .expect_err("fake route fails");
        let second_error = second
            .await
            .expect("second waiter task")
            .expect_err("joined fake route fails");
        assert_eq!(first_error.kind(), DomainErrorKind::AddressUnavailable);
        assert_eq!(second_error.kind(), first_error.kind());
        assert_eq!(second_error.detail(), first_error.detail());
        assert_eq!(transport.connect_calls.load(Ordering::SeqCst), 1);
        assert_eq!(transport.confirmation_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn create_requires_online_route_replays_and_rejects_fingerprint_mismatch() {
        use std::io::{Read as _, Seek as _, SeekFrom};
        let mut capture = tempfile::tempfile().expect("pair log capture");
        let writer = capture.try_clone().expect("pair log writer");
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || writer.try_clone().expect("pair log event writer"))
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let transport = Arc::new(FakeTransport::empty());
        let harness = harness(transport);
        let operation = EphemeralOperationId::from_array([0x61; 16]);
        let fingerprint = PairFingerprint::for_create(DEFAULT_PAIR_TTL_SECONDS);
        let deadline = Instant::now() + Duration::from_secs(2);
        let offline = harness.service.create_until(
            LocalPairCreateInput::new(operation, fingerprint.clone(), 0),
            deadline,
        );
        assert_eq!(
            offline.expect_err("offline create fails").kind(),
            DomainErrorKind::AddressUnavailable
        );

        let mut online = NetworkObservation::disabled(harness.service.inner.identity.device_id());
        online.state = NetworkState::Online;
        online.endpoint_bound = true;
        online.home_relay = Some("https://relay.example.test".to_owned());
        harness.network.set(online);
        let created = harness
            .service
            .create_until(
                LocalPairCreateInput::new(operation, fingerprint.clone(), 0),
                deadline,
            )
            .expect("online create");
        let replayed = harness
            .service
            .create_until(
                LocalPairCreateInput::new(operation, fingerprint, 0),
                deadline,
            )
            .expect("create replay");
        assert_eq!(created.ticket().expose(), replayed.ticket().expose());
        capture
            .seek(SeekFrom::Start(0))
            .expect("read captured pair events");
        let mut logs = String::new();
        capture.read_to_string(&mut logs).expect("pair event text");
        assert_eq!(logs.matches("Pairing ticket created").count(), 1);
        assert!(!logs.contains(created.ticket().expose()));
        assert!(!logs.contains("relay.example.test"));

        let mismatch = harness.service.create_until(
            LocalPairCreateInput::new(
                operation,
                PairFingerprint::for_create(DEFAULT_PAIR_TTL_SECONDS + 1),
                0,
            ),
            deadline,
        );
        assert_eq!(
            mismatch.expect_err("fingerprint mismatch").kind(),
            DomainErrorKind::PairOutcomeUnknown
        );
    }

    #[test]
    fn caller_deadline_is_capped_by_pairing_total_limit() {
        let transport = Arc::new(FakeTransport::empty());
        let harness = harness(transport);
        let before = Instant::now();
        let capped = harness
            .service
            .inner
            .cap_pairing_deadline(before + Duration::from_secs(3_600))
            .expect("deadline cap");
        assert!(
            capped
                <= before
                    + harness.service.inner.limits.pairing_total_deadline
                    + Duration::from_millis(10)
        );
    }

    #[test]
    fn completed_accept_churn_retains_bounded_ticket_free_replay_tombstones() {
        let transport = Arc::new(FakeTransport::empty());
        let harness = harness(transport);
        let maximum = harness.service.inner.limits.max_live_pair_offers;
        let mut operations = Vec::new();
        for index in 0..maximum + 2 {
            let operation_id = EphemeralOperationId::from_array(
                [u8::try_from(index + 1).expect("small test operation"); 16],
            );
            let fingerprint = PairFingerprint::for_create(
                u64::try_from(index + 1).expect("small test fingerprint"),
            );
            let cell = match harness
                .service
                .inner
                .admit_accept(operation_id, &fingerprint)
                .expect("completed cells make room")
            {
                AcceptAdmission::Created(cell) => cell,
                AcceptAdmission::Live(_) | AcceptAdmission::Complete(_) => {
                    panic!("fresh operation must allocate")
                }
            };
            cell.complete(Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                format!("terminal-{index}"),
            )));
            harness
                .service
                .inner
                .record_accept_completion(operation_id, &cell);
            operations.push((operation_id, fingerprint));
        }

        let registry = mutex_lock(&harness.service.inner.accepts);
        assert_eq!(registry.cells.len(), maximum);
        assert!(registry.retired.len() <= maximum);
        assert!(registry.retired.contains_key(&operations[0].0));
        drop(registry);

        let replay = harness
            .service
            .inner
            .existing_accept(operations[0].0, &operations[0].1)
            .expect("same retired fingerprint replays")
            .expect("recent retired operation exists");
        match replay {
            AcceptReplay::Complete(Err(error)) => {
                assert_eq!(error.kind(), DomainErrorKind::Cancelled);
                assert_eq!(error.detail(), "terminal-0");
            }
            AcceptReplay::Live(_) | AcceptReplay::Complete(Ok(_)) => {
                panic!("retired error must replay exactly")
            }
        }

        let mismatch = harness
            .service
            .inner
            .existing_accept(operations[0].0, &PairFingerprint::for_create(9_999));
        let Err(error) = mismatch else {
            panic!("retired ID mismatch must not return a replay");
        };
        assert_eq!(error.kind(), DomainErrorKind::PairOutcomeUnknown);
    }
}
