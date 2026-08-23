//! Typed observation and lifecycle owner for the daemon's sole Iroh Endpoint.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use iroh::endpoint::{Incoming, VarInt};
use iroh::{Endpoint, SecretKey, Watcher};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::{JoinHandle, JoinSet};
use zterm_core::{DeviceId, DomainErrorKind, TransportLimits};

use crate::authorization::AuthorizationRegistry;
use crate::connection_broker::{ConnectionBroker, ConnectionIdentity, PairConnection};
use crate::error::DaemonError;
use crate::identity::DeviceIdentity;
use crate::store::StoreHandle;
use crate::transport::{InfrastructureProfile, ZTERM_ALPN, ZTERM_PAIR_ALPN};

/// Lifecycle state of the sole Iroh endpoint supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkState {
    /// No network owner was attached (isolated service/test construction).
    Disabled,
    /// The supervisor owns the stable identity and is attempting its first bind.
    Initializing,
    /// UDP sockets are bound; Relay reachability is not yet established.
    Bound,
    /// Local service remains available while bind or Relay connectivity is impaired.
    Degraded,
    /// At least one configured home Relay is connected.
    Online,
    /// New accepts and dials are quiesced during final daemon cleanup.
    Stopping,
    /// Endpoint close completed and all network ownership was released.
    Stopped,
}

impl NetworkState {
    /// Stable lowercase status value used by IPC projections.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Initializing => "initializing",
            Self::Bound => "bound",
            Self::Degraded => "degraded",
            Self::Online => "online",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
        }
    }
}

/// Factual configuration/health summary for one address-lookup direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressServiceState {
    /// No service is attached.
    Disabled,
    /// Service is configured; its fire-and-forget operation has no success signal.
    Configured,
    /// The owning Endpoint is currently unavailable.
    Degraded,
}

impl AddressServiceState {
    /// Stable lowercase IPC value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Configured => "configured",
            Self::Degraded => "degraded",
        }
    }
}

/// Redacted diagnostic category for the most recent network degradation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkDiagnostic {
    /// Iroh failed to bind its local sockets.
    EndpointBindFailed,
    /// A bound Endpoint stopped unexpectedly and will be rebound.
    EndpointClosed,
    /// Configured home Relays are presently unreachable.
    HomeRelayUnavailable,
}

impl NetworkDiagnostic {
    /// Stable local-only diagnostic code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::EndpointBindFailed => "endpoint_bind_failed",
            Self::EndpointClosed => "endpoint_closed",
            Self::HomeRelayUnavailable => "home_relay_unavailable",
        }
    }
}

/// Selected network path kind; direct addresses themselves are never exposed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PathKind {
    /// No selected path has been observed.
    #[default]
    Unknown,
    /// Selected path is an IP transport; the address remains redacted.
    Direct,
    /// Selected path uses a Relay.
    Relay,
}

impl PathKind {
    /// Stable lowercase status value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Direct => "direct",
            Self::Relay => "relay",
        }
    }
}

/// Current redacted network observation shared by daemon status and broker users.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkObservation {
    /// Stable public identity retained across bind retries.
    pub device_id: DeviceId,
    /// Current supervisor lifecycle state.
    pub state: NetworkState,
    /// Whether the sole Endpoint currently owns bound sockets.
    pub endpoint_bound: bool,
    /// Number of bind attempts made with this identity.
    pub bind_attempts: u64,
    /// Selected/configured home Relay URL, when Iroh reports one.
    pub home_relay: Option<String>,
    /// Address publication configuration/health summary.
    pub publish: AddressServiceState,
    /// Address resolution configuration/health summary.
    pub lookup: AddressServiceState,
    /// Fully TLS-authenticated provisional and primary connections.
    pub authenticated_connection_count: u32,
    /// Promoted primary peer connections.
    pub primary_connection_count: u32,
    /// Currently open broker-owned business streams.
    pub active_stream_count: u32,
    /// Primary peers whose selected path is direct.
    pub direct_path_count: u32,
    /// Primary peers whose selected path uses a Relay.
    pub relay_path_count: u32,
    /// Most recent redacted degradation, if any.
    pub diagnostic: Option<NetworkDiagnostic>,
}

impl NetworkObservation {
    /// Initial observation before the supervisor's first bind attempt.
    #[must_use]
    pub const fn initializing(device_id: DeviceId) -> Self {
        Self {
            device_id,
            state: NetworkState::Initializing,
            endpoint_bound: false,
            bind_attempts: 0,
            home_relay: None,
            publish: AddressServiceState::Degraded,
            lookup: AddressServiceState::Degraded,
            authenticated_connection_count: 0,
            primary_connection_count: 0,
            active_stream_count: 0,
            direct_path_count: 0,
            relay_path_count: 0,
            diagnostic: None,
        }
    }

    /// Passive observation used by isolated constructors that do not bind Iroh.
    #[must_use]
    pub const fn disabled(device_id: DeviceId) -> Self {
        Self {
            device_id,
            state: NetworkState::Disabled,
            endpoint_bound: false,
            bind_attempts: 0,
            home_relay: None,
            publish: AddressServiceState::Disabled,
            lookup: AddressServiceState::Disabled,
            authenticated_connection_count: 0,
            primary_connection_count: 0,
            active_stream_count: 0,
            direct_path_count: 0,
            relay_path_count: 0,
            diagnostic: None,
        }
    }
}

/// Cloneable read-only watch into network state.
#[derive(Clone, Debug)]
pub struct NetworkObserver {
    receiver: watch::Receiver<NetworkObservation>,
}

impl NetworkObserver {
    /// Creates a passive observer for an isolated service.
    #[must_use]
    pub fn disabled(device_id: DeviceId) -> Self {
        let (_, receiver) = watch::channel(NetworkObservation::disabled(device_id));
        Self { receiver }
    }

    /// Returns the latest observation without waiting or performing network work.
    #[must_use]
    pub fn snapshot(&self) -> NetworkObservation {
        self.receiver.borrow().clone()
    }

    /// Subscribes to future observation changes.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<NetworkObservation> {
        self.receiver.clone()
    }
}

/// Single write boundary shared by the supervisor and connection broker.
#[derive(Clone, Debug)]
pub(crate) struct NetworkReporter {
    sender: watch::Sender<NetworkObservation>,
}

/// Boxed future returned by the narrow inbound pairing callback.
pub type PairConnectionHandlerFuture =
    Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'static>>;

/// Async callback installed after the broker is composed but before bind.
///
/// The callback receives only a fully TLS-authenticated pair connection and
/// the one absolute pairing deadline. It never receives the owning Endpoint,
/// normal peer registry, or infrastructure profile.
pub trait PairConnectionHandler: Send + Sync + 'static {
    /// Handles one inbound `zterm-pair/1` connection.
    fn handle_pair_connection(
        &self,
        connection: PairConnection,
        deadline: Instant,
    ) -> PairConnectionHandlerFuture;
}

impl NetworkReporter {
    pub(crate) fn initializing(device_id: DeviceId) -> (Self, NetworkObserver) {
        let (sender, receiver) = watch::channel(NetworkObservation::initializing(device_id));
        (Self { sender }, NetworkObserver { receiver })
    }

    pub(crate) fn update(&self, update: impl FnOnce(&mut NetworkObservation)) {
        self.sender.send_modify(update);
    }

    pub(crate) fn transport_metrics(
        &self,
        authenticated: usize,
        primary: usize,
        streams: usize,
        direct_paths: usize,
        relay_paths: usize,
    ) {
        self.update(|observation| {
            observation.authenticated_connection_count = bounded_u32(authenticated);
            observation.primary_connection_count = bounded_u32(primary);
            observation.active_stream_count = bounded_u32(streams);
            observation.direct_path_count = bounded_u32(direct_paths);
            observation.relay_path_count = bounded_u32(relay_paths);
        });
    }
}

fn bounded_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Prepared network ownership which has not yet spawned or bound an Endpoint.
pub struct NetworkStartup {
    device_id: DeviceId,
    secret_key: SecretKey,
    profile: InfrastructureProfile,
    limits: TransportLimits,
    broker: ConnectionBroker,
    pre_auth: PreAuthLimits,
    pair_handler: Option<Arc<dyn PairConnectionHandler>>,
    reporter: NetworkReporter,
    shutdown: watch::Receiver<bool>,
    hooks: NetworkTestHooks,
}

impl std::fmt::Debug for NetworkStartup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkStartup")
            .field("device_id", &self.device_id)
            .field("profile", &self.profile)
            .field("limits", &self.limits)
            .field("pair_handler_installed", &self.pair_handler.is_some())
            .finish_non_exhaustive()
    }
}

/// Cloneable access to observation and authenticated connection demand.
#[derive(Clone, Debug)]
pub struct NetworkHandle {
    observer: NetworkObserver,
    broker: ConnectionBroker,
    shutdown: watch::Sender<bool>,
}

impl NetworkHandle {
    /// Returns the current redacted network observation.
    #[must_use]
    pub fn observe(&self) -> NetworkObserver {
        self.observer.clone()
    }

    /// Returns the sole per-peer connection registry.
    #[must_use]
    pub fn broker(&self) -> ConnectionBroker {
        self.broker.clone()
    }
}

/// Runtime owner of the supervisor task and its bounded shutdown join.
pub struct NetworkSupervisor {
    handle: NetworkHandle,
    reporter: NetworkReporter,
    task: Option<JoinHandle<Result<(), DaemonError>>>,
    shutdown_complete: bool,
}

impl std::fmt::Debug for NetworkSupervisor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkSupervisor")
            .field("observation", &self.handle.observer.snapshot())
            .finish_non_exhaustive()
    }
}

/// Controlled failure/backoff hooks for task-private lifecycle tests.
#[derive(Clone, Debug)]
pub struct NetworkTestHooks {
    bind_failures_remaining: Arc<AtomicUsize>,
    retry_base: Duration,
    retry_cap: Duration,
    endpoint_close_count: Arc<AtomicUsize>,
}

impl Default for NetworkTestHooks {
    fn default() -> Self {
        Self {
            bind_failures_remaining: Arc::new(AtomicUsize::new(0)),
            retry_base: Duration::from_millis(250),
            retry_cap: Duration::from_secs(10),
            endpoint_close_count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl NetworkTestHooks {
    /// Creates deterministic bind failures and short retry timing for tests.
    #[doc(hidden)]
    #[must_use]
    pub fn injected_bind_failures(failures: usize, retry: Duration) -> Self {
        Self::injected_bind_failures_with_backoff(failures, retry, retry)
    }

    /// Creates deterministic pre-bind failures with an explicit retry curve.
    #[doc(hidden)]
    #[must_use]
    pub fn injected_bind_failures_with_backoff(
        failures: usize,
        retry_base: Duration,
        retry_cap: Duration,
    ) -> Self {
        Self {
            bind_failures_remaining: Arc::new(AtomicUsize::new(failures)),
            retry_base,
            retry_cap,
            endpoint_close_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns the deterministic retry delay this hook applies to one identity.
    #[doc(hidden)]
    #[must_use]
    pub fn retry_delay_for_test(&self, attempt: u32, device_id: DeviceId) -> Duration {
        supervisor_retry_delay(attempt, device_id, self.retry_base, self.retry_cap)
    }

    /// Number of successful Endpoint close calls observed by this hook.
    #[doc(hidden)]
    #[must_use]
    pub fn endpoint_close_count(&self) -> usize {
        self.endpoint_close_count.load(Ordering::Acquire)
    }

    fn take_bind_failure(&self) -> bool {
        self.bind_failures_remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
    }
}

impl NetworkStartup {
    /// Composes identity, infrastructure, authorization, store, and broker
    /// ownership without binding or performing network I/O.
    pub fn prepare(
        identity: DeviceIdentity,
        profile: InfrastructureProfile,
        connection_identity: ConnectionIdentity,
        store: StoreHandle,
        authorization: AuthorizationRegistry,
        limits: TransportLimits,
    ) -> Result<(Self, NetworkHandle), DaemonError> {
        limits.validate().map_err(|error| {
            DaemonError::new(DomainErrorKind::ResourceExhausted, error.to_string())
        })?;
        let device_id = identity.device_id();
        if connection_identity.device_id() != device_id {
            return Err(DaemonError::new(
                DomainErrorKind::IdentityStateMismatch,
                "network diagnostics identity does not match identity.key",
            ));
        }
        let (reporter, observer) = NetworkReporter::initializing(device_id);
        let broker = ConnectionBroker::with_reporter(
            connection_identity,
            store,
            authorization,
            limits,
            reporter.clone(),
            observer.clone(),
        )?;
        let pre_auth = PreAuthLimits::new(limits, broker.pair_handshake_admission());
        let (shutdown, shutdown_receiver) = watch::channel(false);
        let handle = NetworkHandle {
            observer,
            broker: broker.clone(),
            shutdown,
        };
        Ok((
            Self {
                device_id,
                secret_key: identity.into_secret_key(),
                profile,
                limits,
                broker,
                pre_auth,
                pair_handler: None,
                reporter,
                shutdown: shutdown_receiver,
                hooks: NetworkTestHooks::default(),
            },
            handle,
        ))
    }

    /// Replaces only deterministic failure/timing hooks before spawning.
    #[doc(hidden)]
    #[must_use]
    pub fn with_test_hooks(mut self, hooks: NetworkTestHooks) -> Self {
        self.hooks = hooks;
        self
    }

    /// Installs the later-composed inbound pairing service without exposing
    /// Endpoint ownership or creating a broker/handler ownership cycle.
    #[must_use]
    pub fn with_pair_handler<H>(mut self, handler: H) -> Self
    where
        H: PairConnectionHandler,
    {
        self.pair_handler = Some(Arc::new(handler));
        self
    }

    /// Spawns the single supervisor on the daemon's existing Tokio runtime.
    #[must_use]
    pub fn spawn(self, handle: NetworkHandle) -> NetworkSupervisor {
        let reporter = self.reporter.clone();
        let task = tokio::spawn(self.run());
        NetworkSupervisor {
            handle,
            reporter,
            task: Some(task),
            shutdown_complete: false,
        }
    }

    async fn run(mut self) -> Result<(), DaemonError> {
        let mut attempt = 0_u32;
        loop {
            if *self.shutdown.borrow() {
                self.finish_without_endpoint().await;
                return Ok(());
            }
            let bind_attempts = u64::from(attempt).saturating_add(1);
            self.reporter.update(|observation| {
                observation.bind_attempts = bind_attempts;
                if attempt == 0 {
                    observation.state = NetworkState::Initializing;
                }
            });

            let bound = if self.hooks.take_bind_failure() {
                Err(())
            } else {
                let bind = self
                    .profile
                    .endpoint_builder(self.secret_key.clone())
                    .bind();
                tokio::select! {
                    changed = self.shutdown.changed() => {
                        let _ = changed;
                        self.finish_without_endpoint().await;
                        return Ok(());
                    }
                    result = bind => result.map_err(|_| ()),
                }
            };

            let endpoint = match bound {
                Ok(endpoint) => endpoint,
                Err(()) => {
                    self.reporter.update(|observation| {
                        observation.state = NetworkState::Degraded;
                        observation.endpoint_bound = false;
                        observation.publish = AddressServiceState::Degraded;
                        observation.lookup = AddressServiceState::Degraded;
                        observation.diagnostic = Some(NetworkDiagnostic::EndpointBindFailed);
                    });
                    let delay = supervisor_retry_delay(
                        attempt,
                        self.device_id,
                        self.hooks.retry_base,
                        self.hooks.retry_cap,
                    );
                    attempt = attempt.saturating_add(1);
                    tokio::select! {
                        () = tokio::time::sleep(delay) => continue,
                        changed = self.shutdown.changed() => {
                            let _ = changed;
                            self.finish_without_endpoint().await;
                            return Ok(());
                        }
                    }
                }
            };

            if endpoint.id().as_bytes() != self.device_id.as_bytes() {
                endpoint.close().await;
                return Err(DaemonError::new(
                    DomainErrorKind::IdentityStateMismatch,
                    "Iroh Endpoint rotated identity while binding",
                ));
            }
            self.broker.attach_endpoint(endpoint.clone()).await?;
            let lookups_configured = endpoint
                .address_lookup()
                .is_ok_and(|services| !services.is_empty());
            self.reporter.update(|observation| {
                observation.state = NetworkState::Bound;
                observation.endpoint_bound = true;
                observation.publish = if lookups_configured {
                    AddressServiceState::Configured
                } else {
                    AddressServiceState::Disabled
                };
                observation.lookup = observation.publish;
                observation.diagnostic = None;
            });
            attempt = 0;
            match self.run_bound(endpoint.clone()).await {
                BoundExit::Shutdown => {
                    self.reporter.update(|observation| {
                        observation.state = NetworkState::Stopping;
                    });
                    self.broker.quiesce().await;
                    endpoint.close().await;
                    self.hooks
                        .endpoint_close_count
                        .fetch_add(1, Ordering::AcqRel);
                    self.reporter.update(|observation| {
                        observation.state = NetworkState::Stopped;
                        observation.endpoint_bound = false;
                        observation.home_relay = None;
                        observation.publish = AddressServiceState::Degraded;
                        observation.lookup = AddressServiceState::Degraded;
                    });
                    return Ok(());
                }
                BoundExit::Closed => {
                    self.broker.detach_endpoint().await;
                    self.reporter.update(|observation| {
                        observation.state = NetworkState::Degraded;
                        observation.endpoint_bound = false;
                        observation.home_relay = None;
                        observation.publish = AddressServiceState::Degraded;
                        observation.lookup = AddressServiceState::Degraded;
                        observation.diagnostic = Some(NetworkDiagnostic::EndpointClosed);
                    });
                }
            }
        }
    }

    async fn finish_without_endpoint(&self) {
        self.reporter.update(|observation| {
            observation.state = NetworkState::Stopping;
        });
        self.broker.quiesce().await;
        self.reporter.update(|observation| {
            observation.state = NetworkState::Stopped;
            observation.endpoint_bound = false;
        });
    }

    async fn run_bound(&mut self, endpoint: Endpoint) -> BoundExit {
        if *self.shutdown.borrow() {
            return BoundExit::Shutdown;
        }
        let limits = self.pre_auth.clone();
        let pair_handler = self.pair_handler.clone();
        let mut relay_status = endpoint.home_relay_status();
        apply_relay_status(&self.reporter, relay_status.get());
        let mut handlers = JoinSet::new();

        let exit = loop {
            tokio::select! {
                biased;
                changed = self.shutdown.changed() => {
                    let _ = changed;
                    break BoundExit::Shutdown;
                }
                _ = endpoint.closed() => break BoundExit::Closed,
                status = relay_status.updated() => {
                    if let Ok(status) = status {
                        apply_relay_status(&self.reporter, status);
                    }
                }
                incoming = endpoint.accept() => {
                    let Some(incoming) = incoming else {
                        break BoundExit::Closed;
                    };
                    let Ok(outer) = limits.outer.clone().try_acquire_owned() else {
                        incoming.refuse();
                        continue;
                    };
                    let broker = self.broker.clone();
                    let limits = limits.clone();
                    let pair_handler = pair_handler.clone();
                    let policy = self.limits;
                    handlers.spawn(async move {
                        route_incoming(
                            incoming,
                            outer,
                            limits,
                            broker,
                            pair_handler,
                            policy,
                        ).await;
                    });
                }
                joined = handlers.join_next(), if !handlers.is_empty() => {
                    let _ = joined;
                }
            }
        };
        handlers.abort_all();
        while handlers.join_next().await.is_some() {}
        exit
    }
}

impl NetworkSupervisor {
    /// Cloneable observation/broker handle owned by this supervisor.
    #[must_use]
    pub fn handle(&self) -> NetworkHandle {
        self.handle.clone()
    }

    /// Quiesces and joins Endpoint close within one absolute lifecycle deadline.
    pub async fn shutdown_until(&mut self, deadline: Instant) -> Result<(), DaemonError> {
        if self.shutdown_complete {
            return Ok(());
        }
        let Some(mut task) = self.task.take() else {
            return Err(DaemonError::new(
                DomainErrorKind::TransportUnavailable,
                "network shutdown did not complete",
            ));
        };
        self.handle.shutdown.send_replace(true);
        self.reporter.update(|observation| {
            observation.state = NetworkState::Stopping;
        });
        if Instant::now() >= deadline {
            task.abort();
            let _ = task.await;
            return Err(DaemonError::new(
                DomainErrorKind::DeadlineExceeded,
                "network shutdown deadline elapsed before Endpoint close",
            ));
        }
        let result = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            &mut task,
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(DaemonError::new(
                DomainErrorKind::TransportUnavailable,
                "network supervisor task ended unexpectedly",
            )),
            Err(_) => {
                task.abort();
                let _ = task.await;
                Err(DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    "network shutdown timed out while closing Endpoint",
                ))
            }
        };
        if result.is_ok() {
            self.shutdown_complete = true;
        }
        result
    }
}

impl Drop for NetworkSupervisor {
    fn drop(&mut self) {
        self.handle.shutdown.send_replace(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Clone)]
struct PreAuthLimits {
    outer: Arc<Semaphore>,
    normal: Arc<Semaphore>,
    normal_per_peer: Arc<PeerPermitMap>,
    pairing: PairHandshakeAdmission,
}

impl PreAuthLimits {
    fn new(limits: TransportLimits, pairing: PairHandshakeAdmission) -> Self {
        Self {
            outer: pairing.outer(),
            normal: Arc::new(Semaphore::new(limits.max_unauthenticated_connections)),
            normal_per_peer: Arc::new(PeerPermitMap::new(limits.max_unauthenticated_per_endpoint)),
            pairing,
        }
    }
}

/// One global/per-endpoint admission owner shared by inbound and outbound
/// pairing handshakes.
#[derive(Clone)]
pub(crate) struct PairHandshakeAdmission {
    inner: Arc<PairHandshakeAdmissionInner>,
}

struct PairHandshakeAdmissionInner {
    outer: Arc<Semaphore>,
    global: Arc<Semaphore>,
    per_peer: Arc<PeerPermitMap>,
    quiescing: watch::Sender<bool>,
}

/// Cancellation-safe RAII ownership for one complete pairing handshake.
pub(crate) struct PairHandshakePermit {
    _outer: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
    _peer: PeerPermit,
}

/// Global admission acquired before an inbound TLS handshake reveals its peer.
pub(crate) struct PairHandshakeGlobalPermit {
    admission: PairHandshakeAdmission,
    deadline: Instant,
    outer: OwnedSemaphorePermit,
    global: OwnedSemaphorePermit,
}

impl PairHandshakeAdmission {
    pub(crate) fn new(limits: TransportLimits) -> Result<Self, DaemonError> {
        let outer_limit = limits
            .max_unauthenticated_connections
            .checked_add(limits.max_pairing_handshakes)
            .ok_or_else(|| {
                DaemonError::new(
                    DomainErrorKind::ResourceExhausted,
                    "pre-authentication connection limit overflow",
                )
            })?;
        let (quiescing, _) = watch::channel(false);
        Ok(Self {
            inner: Arc::new(PairHandshakeAdmissionInner {
                outer: Arc::new(Semaphore::new(outer_limit)),
                global: Arc::new(Semaphore::new(limits.max_pairing_handshakes)),
                per_peer: Arc::new(PeerPermitMap::new(limits.max_pairing_per_endpoint)),
                quiescing,
            }),
        })
    }

    fn outer(&self) -> Arc<Semaphore> {
        Arc::clone(&self.inner.outer)
    }

    /// Acquires the shared outer/global/per-peer permits for an outbound dial.
    pub(crate) async fn acquire(
        &self,
        remote: DeviceId,
        deadline: Instant,
    ) -> Result<PairHandshakePermit, DaemonError> {
        // Pair-global admission comes first so queued outbound pair work never
        // occupies the capacity reserved for normal inbound pre-authentication.
        let global = self
            .acquire_semaphore(
                Arc::clone(&self.inner.global),
                deadline,
                "global pairing handshake limit reached",
            )
            .await?;
        let outer = self
            .acquire_semaphore(
                Arc::clone(&self.inner.outer),
                deadline,
                "pairing pre-authentication limit reached",
            )
            .await?;
        PairHandshakeGlobalPermit {
            admission: self.clone(),
            deadline,
            outer,
            global,
        }
        .bind_peer(remote)
    }

    /// Acquires the pair-global permit while reusing the inbound accept's
    /// already-held outer permit.
    pub(crate) async fn begin_with_outer(
        &self,
        outer: OwnedSemaphorePermit,
        deadline: Instant,
    ) -> Result<PairHandshakeGlobalPermit, DaemonError> {
        if *self.inner.quiescing.borrow() {
            return Err(cancelled("pairing transport is stopping"));
        }
        if Instant::now() >= deadline {
            return Err(deadline_exceeded("pairing admission deadline elapsed"));
        }
        // An accepted socket already owns outer admission. Reject immediately
        // when the pair category is full instead of letting unauthenticated
        // pair waiters consume normal protocol capacity.
        let global = self
            .inner
            .global
            .clone()
            .try_acquire_owned()
            .map_err(|_| resource_exhausted("global pairing handshake limit reached"))?;
        Ok(PairHandshakeGlobalPermit {
            admission: self.clone(),
            deadline,
            outer,
            global,
        })
    }

    async fn acquire_semaphore(
        &self,
        semaphore: Arc<Semaphore>,
        deadline: Instant,
        overloaded: &'static str,
    ) -> Result<OwnedSemaphorePermit, DaemonError> {
        let mut quiescing = self.subscribe();
        if *quiescing.borrow() {
            return Err(cancelled("pairing transport is stopping"));
        }
        if Instant::now() >= deadline {
            return Err(deadline_exceeded("pairing admission deadline elapsed"));
        }
        tokio::select! {
            biased;
            changed = quiescing.changed() => {
                let _ = changed;
                Err(cancelled("pairing transport is stopping"))
            }
            result = tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                semaphore.acquire_owned(),
            ) => match result {
                Ok(Ok(permit)) => Ok(permit),
                Ok(Err(_)) => Err(transport_unavailable("pairing admission is unavailable")),
                Err(_) => Err(resource_exhausted(overloaded)),
            }
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.inner.quiescing.subscribe()
    }

    pub(crate) fn begin_quiesce(&self) {
        self.inner.quiescing.send_replace(true);
    }
}

impl PairHandshakeGlobalPermit {
    /// Adds the exact TLS-authenticated peer ownership after inbound TLS.
    pub(crate) fn bind_peer(self, remote: DeviceId) -> Result<PairHandshakePermit, DaemonError> {
        if *self.admission.inner.quiescing.borrow() {
            return Err(cancelled("pairing transport is stopping"));
        }
        if Instant::now() >= self.deadline {
            return Err(deadline_exceeded("pairing admission deadline elapsed"));
        }
        let peer = self
            .admission
            .inner
            .per_peer
            .try_acquire(remote)
            .ok_or_else(|| resource_exhausted("per-endpoint pairing handshake limit reached"))?;
        Ok(PairHandshakePermit {
            _outer: self.outer,
            _global: self.global,
            _peer: peer,
        })
    }
}

struct PeerPermitMap {
    maximum: usize,
    counts: Mutex<BTreeMap<DeviceId, usize>>,
}

impl PeerPermitMap {
    fn new(maximum: usize) -> Self {
        Self {
            maximum,
            counts: Mutex::new(BTreeMap::new()),
        }
    }

    fn try_acquire(self: &Arc<Self>, remote: DeviceId) -> Option<PeerPermit> {
        let mut counts = mutex_lock(&self.counts);
        let count = counts.entry(remote).or_default();
        if *count >= self.maximum {
            return None;
        }
        *count = count.checked_add(1)?;
        Some(PeerPermit {
            owner: Arc::clone(self),
            remote,
        })
    }
}

struct PeerPermit {
    owner: Arc<PeerPermitMap>,
    remote: DeviceId,
}

impl Drop for PeerPermit {
    fn drop(&mut self) {
        let mut counts = mutex_lock(&self.owner.counts);
        if let Some(count) = counts.get_mut(&self.remote) {
            let Some(next) = count.checked_sub(1) else {
                return;
            };
            *count = next;
            if *count == 0 {
                counts.remove(&self.remote);
            }
        }
    }
}

enum BoundExit {
    Shutdown,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IncomingProtocol {
    Normal,
    Pairing,
}

fn classify_incoming_alpn(alpn: &[u8]) -> Option<IncomingProtocol> {
    if alpn == ZTERM_ALPN {
        Some(IncomingProtocol::Normal)
    } else if alpn == ZTERM_PAIR_ALPN {
        Some(IncomingProtocol::Pairing)
    } else {
        None
    }
}

async fn route_incoming(
    incoming: Incoming,
    outer: OwnedSemaphorePermit,
    limits: PreAuthLimits,
    broker: ConnectionBroker,
    pair_handler: Option<Arc<dyn PairConnectionHandler>>,
    policy: TransportLimits,
) {
    let started = Instant::now();
    let normal_deadline = started
        .checked_add(policy.connect_attempt_budget)
        .unwrap_or(started);
    let pair_deadline = started
        .checked_add(policy.pairing_total_deadline)
        .unwrap_or(started);
    let Ok(mut accepting) = incoming.accept() else {
        return;
    };
    let alpn = match timeout_at(normal_deadline, accepting.alpn()).await {
        Ok(Ok(alpn)) => alpn,
        _ => return,
    };
    match classify_incoming_alpn(&alpn) {
        Some(IncomingProtocol::Normal) => {
            let Ok(_category) = limits.normal.clone().try_acquire_owned() else {
                return;
            };
            // Awaiting Accepting completes mutual TLS authentication. The
            // completed Connection type cannot be produced by Iroh's 0-RTT API.
            let connection = match timeout_at(normal_deadline, accepting).await {
                Ok(Ok(connection)) => connection,
                _ => return,
            };
            let remote = DeviceId::from_array(*connection.remote_id().as_bytes());
            let Some(_peer) = limits.normal_per_peer.try_acquire(remote) else {
                connection.close(VarInt::from_u32(0x104), b"transport overloaded");
                return;
            };
            let _outer = outer;
            let _ = broker.accept_normal(connection).await;
        }
        Some(IncomingProtocol::Pairing) => {
            let global = match limits.pairing.begin_with_outer(outer, pair_deadline).await {
                Ok(global) => global,
                Err(_) => return,
            };
            // The same absolute pairing deadline includes ALPN selection, full
            // mutual TLS, admission, and the application handshake callback.
            let connection = match timeout_or_pair_shutdown(
                pair_deadline,
                limits.pairing.subscribe(),
                accepting,
            )
            .await
            {
                Ok(Ok(connection)) => connection,
                _ => return,
            };
            let remote = DeviceId::from_array(*connection.remote_id().as_bytes());
            let permit = match global.bind_peer(remote) {
                Ok(permit) => permit,
                Err(_) => {
                    connection.close(VarInt::from_u32(0x104), b"transport overloaded");
                    return;
                }
            };
            let Some(handler) = pair_handler else {
                connection.close(VarInt::from_u32(0x101), b"pairing unavailable");
                return;
            };
            let pair = match broker.pair_from_incoming(connection, permit) {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let _ = handler.handle_pair_connection(pair, pair_deadline).await;
        }
        None => {}
    }
}

fn apply_relay_status(reporter: &NetworkReporter, statuses: Vec<iroh::endpoint::RelayStatus>) {
    let connected = statuses.iter().find(|status| status.is_connected());
    let selected = connected.or_else(|| statuses.first());
    let failed = connected.is_none() && statuses.iter().any(|status| status.last_error().is_some());
    reporter.update(|observation| {
        observation.home_relay = selected.map(|status| status.url().to_string());
        if connected.is_some() {
            observation.state = NetworkState::Online;
            observation.diagnostic = None;
        } else if failed {
            observation.state = NetworkState::Degraded;
            observation.diagnostic = Some(NetworkDiagnostic::HomeRelayUnavailable);
        } else {
            observation.state = NetworkState::Bound;
            observation.diagnostic = None;
        }
    });
}

fn supervisor_retry_delay(
    attempt: u32,
    device_id: DeviceId,
    base: Duration,
    cap: Duration,
) -> Duration {
    let exponent = attempt.min(6);
    let scaled = base.saturating_mul(1_u32 << exponent).min(cap);
    let bytes = device_id.as_bytes();
    let mixed =
        u32::from(bytes[(attempt as usize) % bytes.len()]) ^ attempt.wrapping_mul(0x85eb_ca6b);
    let ceiling = scaled / 5;
    let jitter = if ceiling.is_zero() {
        Duration::ZERO
    } else {
        Duration::from_nanos(
            u64::from(mixed) % u64::try_from(ceiling.as_nanos()).unwrap_or(u64::MAX),
        )
    };
    scaled.saturating_add(jitter).min(cap)
}

async fn timeout_at<F>(deadline: Instant, future: F) -> Result<F::Output, DaemonError>
where
    F: Future,
{
    if Instant::now() >= deadline {
        return Err(deadline_exceeded("network handshake deadline elapsed"));
    }
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future)
        .await
        .map_err(|_| deadline_exceeded("network handshake deadline elapsed"))
}

async fn timeout_or_pair_shutdown<F>(
    deadline: Instant,
    mut shutdown: watch::Receiver<bool>,
    future: F,
) -> Result<F::Output, DaemonError>
where
    F: Future,
{
    if *shutdown.borrow() {
        return Err(cancelled("pairing transport is stopping"));
    }
    if Instant::now() >= deadline {
        return Err(deadline_exceeded("pairing handshake deadline elapsed"));
    }
    tokio::select! {
        biased;
        changed = shutdown.changed() => {
            let _ = changed;
            Err(cancelled("pairing transport is stopping"))
        }
        result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future) => {
            result.map_err(|_| deadline_exceeded("pairing handshake deadline elapsed"))
        }
    }
}

fn resource_exhausted(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::ResourceExhausted, detail)
}

fn transport_unavailable(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::TransportUnavailable, detail)
}

fn deadline_exceeded(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::DeadlineExceeded, detail)
}

fn cancelled(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::Cancelled, detail)
}

fn mutex_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compact_limits() -> TransportLimits {
        TransportLimits {
            max_unauthenticated_connections: 2,
            max_unauthenticated_per_endpoint: 1,
            max_pairing_handshakes: 1,
            max_pairing_per_endpoint: 1,
            ..TransportLimits::default()
        }
    }

    fn pre_auth(limits: TransportLimits) -> PreAuthLimits {
        let pairing = PairHandshakeAdmission::new(limits).expect("pair admission");
        PreAuthLimits::new(limits, pairing)
    }

    #[test]
    fn bind_retry_is_capped_and_stable_for_one_identity() {
        let device = DeviceId::from_array([0x42; 32]);
        let values = (0..12)
            .map(|attempt| {
                supervisor_retry_delay(
                    attempt,
                    device,
                    Duration::from_millis(250),
                    Duration::from_secs(10),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(values, {
            (0..12)
                .map(|attempt| {
                    supervisor_retry_delay(
                        attempt,
                        device,
                        Duration::from_millis(250),
                        Duration::from_secs(10),
                    )
                })
                .collect::<Vec<_>>()
        });
        assert!(values.iter().all(|delay| *delay <= Duration::from_secs(10)));
        assert!(values[0] >= Duration::from_millis(250));
    }

    #[test]
    fn per_peer_permit_is_exact_and_released_by_drop() {
        let owner = Arc::new(PeerPermitMap::new(1));
        let remote = DeviceId::from_array([7; 32]);
        let permit = owner.try_acquire(remote).expect("first permit");
        assert!(owner.try_acquire(remote).is_none());
        drop(permit);
        assert!(owner.try_acquire(remote).is_some());
    }

    #[tokio::test]
    async fn pre_auth_global_and_protocol_budgets_are_bounded_and_separate() {
        let limits = pre_auth(compact_limits());

        let outer_one = limits.outer.clone().try_acquire_owned().expect("outer one");
        let outer_two = limits.outer.clone().try_acquire_owned().expect("outer two");
        let outer_three = limits
            .outer
            .clone()
            .try_acquire_owned()
            .expect("outer three");
        assert!(limits.outer.clone().try_acquire_owned().is_err());

        let normal_one = limits
            .normal
            .clone()
            .try_acquire_owned()
            .expect("normal one");
        let normal_two = limits
            .normal
            .clone()
            .try_acquire_owned()
            .expect("normal two");
        assert!(limits.normal.clone().try_acquire_owned().is_err());
        drop((outer_one, outer_two, outer_three));
        drop((normal_one, normal_two));
        assert_eq!(limits.outer.available_permits(), 3);
        assert_eq!(limits.normal.available_permits(), 2);

        let pairing = limits
            .pairing
            .acquire(
                DeviceId::from_array([0x21; 32]),
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .expect("pairing remains available when normal is exhausted");
        assert_eq!(limits.pairing.inner.global.available_permits(), 0);
        drop(pairing);
        assert_eq!(limits.pairing.inner.global.available_permits(), 1);
    }

    #[tokio::test]
    async fn pre_auth_per_peer_budgets_are_separate_and_release_exactly() {
        let limits = pre_auth(compact_limits());
        let remote = DeviceId::from_array([0x31; 32]);
        let other = DeviceId::from_array([0x32; 32]);

        let normal = limits
            .normal_per_peer
            .try_acquire(remote)
            .expect("normal peer permit");
        assert!(limits.normal_per_peer.try_acquire(remote).is_none());
        assert!(limits.normal_per_peer.try_acquire(other).is_some());

        let pairing = limits
            .pairing
            .acquire(remote, Instant::now() + Duration::from_secs(1))
            .await
            .expect("pairing uses an independent peer budget");
        assert!(limits.pairing.inner.per_peer.try_acquire(remote).is_none());

        drop((normal, pairing));
        assert!(limits.normal_per_peer.try_acquire(remote).is_some());
        assert!(limits.pairing.inner.per_peer.try_acquire(remote).is_some());
    }

    #[tokio::test]
    async fn inbound_and_outbound_pairing_share_eight_global_and_one_peer() {
        let limits = TransportLimits::default();
        let admission = PairHandshakeAdmission::new(limits).expect("pair admission");
        let deadline = Instant::now() + Duration::from_secs(1);

        let shared = DeviceId::from_array([0x30; 32]);
        let outbound = admission
            .acquire(shared, deadline)
            .await
            .expect("outbound permit");
        let inbound_outer = admission
            .outer()
            .try_acquire_owned()
            .expect("inbound outer permit");
        let inbound_global = admission
            .begin_with_outer(inbound_outer, deadline)
            .await
            .expect("inbound global permit");
        let same_peer = match inbound_global.bind_peer(shared) {
            Ok(_) => panic!("inbound and outbound must share one per-peer budget"),
            Err(error) => error,
        };
        assert_eq!(same_peer.kind(), DomainErrorKind::ResourceExhausted);

        let inbound_outer = admission
            .outer()
            .try_acquire_owned()
            .expect("second inbound outer permit");
        let inbound = admission
            .begin_with_outer(inbound_outer, deadline)
            .await
            .expect("second inbound global permit")
            .bind_peer(DeviceId::from_array([0x31; 32]))
            .expect("different inbound peer");
        let mut permits = vec![outbound, inbound];
        for byte in 0x32..0x38 {
            permits.push(
                admission
                    .acquire(DeviceId::from_array([byte; 32]), deadline)
                    .await
                    .expect("remaining global permit"),
            );
        }
        assert_eq!(permits.len(), 8);
        assert_eq!(admission.inner.global.available_permits(), 0);
        assert_eq!(
            admission.inner.outer.available_permits(),
            limits.max_unauthenticated_connections,
            "pair handshakes leave the normal pre-authentication share available"
        );

        drop(permits);
        assert_eq!(admission.inner.global.available_permits(), 8);
    }

    #[tokio::test]
    async fn pair_admission_quiesce_cancels_a_pending_outbound_handshake() {
        let policy = compact_limits();
        let admission = PairHandshakeAdmission::new(policy).expect("pair admission");
        let held = admission
            .acquire(
                DeviceId::from_array([0x61; 32]),
                Instant::now() + Duration::from_secs(5),
            )
            .await
            .expect("first handshake owns the global permit");
        let waiting_admission = admission.clone();
        let waiting = tokio::spawn(async move {
            waiting_admission
                .acquire(
                    DeviceId::from_array([0x62; 32]),
                    Instant::now() + Duration::from_secs(5),
                )
                .await
        });
        tokio::task::yield_now().await;
        admission.begin_quiesce();
        let error = match waiting.await.expect("waiter task") {
            Ok(_) => panic!("quiesce must cancel pending pair admission"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), DomainErrorKind::Cancelled);
        drop(held);
    }

    #[test]
    fn incoming_alpn_routes_only_exact_normal_and_pairing_protocols() {
        assert_eq!(
            classify_incoming_alpn(ZTERM_ALPN),
            Some(IncomingProtocol::Normal)
        );
        assert_eq!(
            classify_incoming_alpn(ZTERM_PAIR_ALPN),
            Some(IncomingProtocol::Pairing)
        );
        assert_eq!(classify_incoming_alpn(b"zterm/1\0"), None);
        assert_eq!(classify_incoming_alpn(b"zterm-pair/2"), None);
    }
}
