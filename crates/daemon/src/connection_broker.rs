//! Per-peer Iroh connection singleflight, duplicate arbitration, and stream admission.

use std::collections::BTreeMap;
use std::fmt;
#[cfg(unix)]
use std::future::Future;
use std::future::pending;
#[cfg(unix)]
use std::pin::Pin;
#[cfg(unix)]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use iroh::endpoint::{Connection, PathEvent, RecvStream, SendStream, VarInt};
use iroh::{Endpoint, EndpointAddr, TransportAddr};
use ring::rand::{SecureRandom, SystemRandom};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{Mutex as AsyncMutex, Notify, OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::{JoinHandle, JoinSet};
use zterm_core::{
    AuthGeneration, AuthorizationSnapshot, AuthorizationStatus, Capabilities, ConnectionAttemptId,
    ConnectionCandidateKey, ConnectionHello, ConnectionWelcome, DeviceDisplayName, DeviceId,
    DomainErrorKind, RelayHint, TransportLimits, designated_primary,
};
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v2};

use crate::authorization::AuthorizationRegistry;
use crate::error::DaemonError;
use crate::network::{
    NetworkObservation, NetworkObserver, NetworkReporter, PairHandshakeAdmission,
    PairHandshakePermit, PathKind,
};
use crate::route::{RouteResolver, device_from_endpoint_id, endpoint_id_from_device};
use crate::store::{RelayRouteCache, StoreHandle};
use crate::transport::{ZTERM_ALPN, ZTERM_PAIR_ALPN};

const CLOSE_UNAUTHORIZED: u32 = 0x100;
const CLOSE_INCOMPATIBLE: u32 = 0x101;
const CLOSE_DUPLICATE: u32 = 0x102;
const CLOSE_SHUTTING_DOWN: u32 = 0x103;
const CLOSE_OVERLOADED: u32 = 0x104;
const CLOSE_PAIR_COMPLETE: u32 = 0x105;
const STREAM_REJECTED: u32 = 0x200;
const RETRY_BASE: Duration = Duration::from_millis(250);
const RETRY_CAP: Duration = Duration::from_secs(10);

/// Stable local diagnostics placed in every normal connection handshake.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionIdentity {
    device_id: DeviceId,
    display_name: DeviceDisplayName,
    build: String,
    platform: String,
    capabilities: Capabilities,
}

impl ConnectionIdentity {
    /// Validates local display/build/platform fields once at composition time.
    pub fn new(
        device_id: DeviceId,
        display_name: impl Into<String>,
        build: impl Into<String>,
        platform: impl Into<String>,
        capabilities: Capabilities,
    ) -> Result<Self, DaemonError> {
        let display_name = DeviceDisplayName::new(display_name).map_err(|error| {
            DaemonError::new(
                DomainErrorKind::IdentityInvalid,
                format!("invalid local device display name: {error}"),
            )
        })?;
        let build = build.into();
        let platform = platform.into();
        // Reuse the domain handshake constructor as the single text boundary.
        ConnectionHello::new(
            zterm_proto::WIRE_MAJOR,
            zterm_proto::WIRE_MAJOR,
            capabilities,
            ConnectionAttemptId::from_array([1; 16]),
            display_name.as_str(),
            build.clone(),
            platform.clone(),
        )
        .map_err(|error| {
            DaemonError::new(
                DomainErrorKind::IdentityInvalid,
                format!("invalid local connection diagnostics: {error}"),
            )
        })?;
        Ok(Self {
            device_id,
            display_name,
            build,
            platform,
            capabilities,
        })
    }

    /// Product-default local diagnostics.
    pub fn product(
        device_id: DeviceId,
        display_name: impl Into<String>,
    ) -> Result<Self, DaemonError> {
        Self::new(
            device_id,
            display_name,
            env!("CARGO_PKG_VERSION"),
            format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
            Capabilities::from_bits_retain(
                Capabilities::LOCAL_LIFECYCLE
                    | Capabilities::SESSION_SERVICE
                    | Capabilities::TERMINAL_SERVICE,
            ),
        )
    }

    /// Stable public device ID.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Validated local display name used by pairing and normal handshakes.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.display_name.as_str()
    }

    /// Stable product build string used only for peer diagnostics.
    #[must_use]
    pub fn build(&self) -> &str {
        &self.build
    }
}

/// Why the broker is intentionally closing a peer connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionCloseReason {
    /// Current local authorization no longer admits the peer.
    Unauthorized,
    /// A lower deterministic candidate replaced this connection.
    Duplicate,
    /// The daemon is quiescing its network owner.
    ShuttingDown,
    /// The sole Endpoint was lost and will be rebound.
    EndpointReset,
}

impl ConnectionCloseReason {
    const fn code(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Duplicate => "duplicate",
            Self::ShuttingDown => "shutting_down",
            Self::EndpointReset => "endpoint_reset",
        }
    }

    const fn wire(self) -> (VarInt, &'static [u8]) {
        match self {
            Self::Unauthorized => (VarInt::from_u32(CLOSE_UNAUTHORIZED), b"not authorized"),
            Self::Duplicate => (VarInt::from_u32(CLOSE_DUPLICATE), b"duplicate connection"),
            Self::ShuttingDown => (VarInt::from_u32(CLOSE_SHUTTING_DOWN), b"transport stopping"),
            Self::EndpointReset => (
                VarInt::from_u32(CLOSE_SHUTTING_DOWN),
                b"transport restarting",
            ),
        }
    }
}

/// Semantic owner requesting a new bidirectional stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamPurpose {
    /// A future M7 Session/terminal service stream.
    Service,
    /// Normal-ALPN confirmation after a pairing outcome.
    AuthorizationConfirmation,
}

/// Redacted live state for one peer slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerConnectionObservation {
    /// Promoted deterministic candidate, when connected.
    pub primary: Option<ConnectionCandidateKey>,
    /// Number of provisional plus primary candidates.
    pub candidate_count: usize,
    /// Number of current RAII consumers.
    pub demand_count: usize,
    /// Number of currently open outbound or accepted service streams.
    pub active_stream_count: u32,
    /// Latest generation with which the remote receiver accepted this host.
    pub remote_acceptance_generation: Option<AuthGeneration>,
    /// Redacted selected path kind.
    pub path: PathKind,
}

/// Pure duplicate-arbitration evidence exposed to named integration gates.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuplicateConnectionTestEvidence {
    /// Candidate selected by both registration schedules.
    pub primary: Option<ConnectionCandidateKey>,
    /// Candidates retained after duplicate cleanup.
    pub remaining_candidate_count: usize,
    /// Duplicate candidates returned to the caller for connection-local close.
    pub loser_count: usize,
    /// Whether a provisional candidate prevented a redundant outbound dial.
    pub redial_suppressed_while_provisional: bool,
    /// Whether receiver authorization prevented redial after promotion.
    pub redial_suppressed_after_confirmation: bool,
    /// Whether peer-close cleanup removed the final candidate and primary.
    pub empty_after_peer_close: bool,
}

/// Socket-free stream-admission and RAII evidence for named integration gates.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamLimitTestEvidence {
    /// Global handler admission rejected a second concurrent holder.
    pub global_overflow_rejected: bool,
    /// Dropping the global permit restored its capacity.
    pub global_capacity_released: bool,
    /// One peer's open-stream queue rejected its own overflow.
    pub peer_overflow_rejected: bool,
    /// A full peer queue did not consume another peer's permit.
    pub peer_isolated: bool,
    /// Dropping peer queue permits restored both peers.
    pub peer_capacity_released: bool,
    /// Per-connection stream and handler limits rejected overflow.
    pub connection_overflow_rejected: bool,
    /// A full connection did not consume another connection's permits.
    pub connection_isolated: bool,
    /// Dropping connection permits restored stream and handler capacity.
    pub connection_capacity_released: bool,
    /// Per-peer metric guards remained independent while both were live.
    pub metric_peer_isolated: bool,
    /// Dropping every metric guard restored the aggregate observation.
    pub metric_capacity_released: bool,
}

/// Redacted path projection evidence exposed to named integration gates.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathObservationTestEvidence {
    /// Path kinds observed in order, followed by `Unknown` after cleanup.
    pub timeline: Vec<PathKind>,
    /// Persistable relay hints for each input; direct IP paths always yield `None`.
    pub persistable_relays: Vec<Option<RelayHint>>,
    /// Aggregate metrics after the final selected input path.
    pub selected_observation: NetworkObservation,
    /// Aggregate metrics after peer-path cleanup.
    pub cleared_observation: NetworkObservation,
}

/// Sole registry of primary Iroh connections for all remote devices.
#[derive(Clone)]
pub struct ConnectionBroker {
    inner: Arc<BrokerInner>,
}

/// Boxed future returned by the narrow inbound normal-service callback.
#[cfg(unix)]
pub(crate) type RemoteServiceHandlerFuture =
    Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'static>>;

/// Object-safe owner of one authenticated inbound normal service stream.
///
/// The callback cannot reach Endpoint, candidate, route, profile, or peer-slot
/// state. It receives only the owned stream, verified peer identity, accepted
/// receiver generation, and the first-frame deadline.
#[cfg(unix)]
pub(crate) trait RemoteServiceHandler: Send + Sync + 'static {
    fn handle_service_stream(
        &self,
        stream: InboundAuthenticatedStream,
        first_frame_deadline: Instant,
    ) -> RemoteServiceHandlerFuture;
}

/// Owned Iroh halves and receiver-owned authorization identity for one
/// inbound normal service stream.
#[cfg(unix)]
pub(crate) struct InboundAuthenticatedStream {
    send: SendStream,
    recv: RecvStream,
    remote_device_id: DeviceId,
    accepted_generation: AuthGeneration,
}

#[cfg(unix)]
impl fmt::Debug for InboundAuthenticatedStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InboundAuthenticatedStream")
            .field("remote_device_id", &self.remote_device_id)
            .field("accepted_generation", &self.accepted_generation)
            .finish_non_exhaustive()
    }
}

#[cfg(unix)]
impl InboundAuthenticatedStream {
    #[must_use]
    pub(crate) const fn remote_device_id(&self) -> DeviceId {
        self.remote_device_id
    }

    #[must_use]
    pub(crate) const fn accepted_generation(&self) -> AuthGeneration {
        self.accepted_generation
    }

    pub(crate) fn into_parts(self) -> (SendStream, RecvStream) {
        (self.send, self.recv)
    }
}

#[derive(Default)]
#[cfg(unix)]
struct RemoteServiceHandlerSlot {
    handler: OnceLock<Arc<dyn RemoteServiceHandler>>,
}

#[cfg(unix)]
impl RemoteServiceHandlerSlot {
    fn install(&self, handler: Arc<dyn RemoteServiceHandler>) -> Result<(), DaemonError> {
        self.handler.set(handler).map_err(|_| {
            DaemonError::new(
                DomainErrorKind::IdentityStateMismatch,
                "remote service handler is already installed",
            )
        })
    }

    fn get(&self) -> Option<Arc<dyn RemoteServiceHandler>> {
        self.handler.get().cloned()
    }

    fn is_installed(&self) -> bool {
        self.handler.get().is_some()
    }
}

impl fmt::Debug for ConnectionBroker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("ConnectionBroker");
        debug.field("local_device_id", &self.inner.identity.device_id);
        #[cfg(unix)]
        debug.field(
            "service_handler_installed",
            &self.inner.service_handler.is_installed(),
        );
        debug.finish_non_exhaustive()
    }
}

struct BrokerInner {
    identity: ConnectionIdentity,
    store: StoreHandle,
    authorization: AuthorizationRegistry,
    resolver: RouteResolver,
    limits: TransportLimits,
    endpoint: watch::Sender<Option<Endpoint>>,
    peers: Mutex<BTreeMap<DeviceId, Arc<PeerSlot>>>,
    admission: BrokerAdmission,
    metrics: Arc<BrokerMetrics>,
    observer: NetworkObserver,
    lifecycle: BrokerLifecycle,
    pairing: PairHandshakeAdmission,
    #[cfg(unix)]
    service_handler: RemoteServiceHandlerSlot,
    test_routes: Mutex<BTreeMap<DeviceId, EndpointAddr>>,
}

struct PeerSlot {
    remote: DeviceId,
    state: AsyncMutex<PeerState>,
    changed: Notify,
    demand: DemandState,
    admission: PeerAdmission,
    active_streams: Arc<AtomicUsize>,
}

#[derive(Default)]
struct PeerState {
    candidates: CandidateRegistry<Arc<Candidate>>,
    remote_acceptance: Option<AuthGeneration>,
    dial_worker_running: bool,
    terminal_error: Option<DaemonError>,
}

struct CandidateRegistry<T> {
    entries: BTreeMap<ConnectionCandidateKey, CandidateEntry<T>>,
    primary: Option<ConnectionCandidateKey>,
}

struct CandidateEntry<T> {
    value: T,
}

enum CandidateDecision<T> {
    Promoted(Vec<T>),
    Lost(T),
    Wait,
    Missing,
}

struct DemandState {
    count: AtomicUsize,
    transient_routes: Mutex<TransientRouteLeases>,
}

#[derive(Default)]
struct TransientRouteLeases {
    ordered: Vec<(RelayHint, usize)>,
}

struct BrokerAdmission {
    pending_dials: Arc<Semaphore>,
    authenticated_connections: Arc<Semaphore>,
    global_stream_handlers: Arc<Semaphore>,
}

struct PeerAdmission {
    open_queue: Arc<Semaphore>,
}

struct CandidateAdmission {
    streams: Arc<Semaphore>,
    handlers: Arc<Semaphore>,
}

#[derive(Debug)]
struct ServiceHandlerPermits {
    _connection: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

#[derive(Default)]
struct BrokerLifecycle {
    shutting_down: AtomicBool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CandidateSide {
    Outbound,
    Inbound,
}

struct Candidate {
    key: ConnectionCandidateKey,
    remote: DeviceId,
    connection: Connection,
    side: CandidateSide,
    inbound_authorization: Option<AuthorizationSnapshot>,
    verified_relay: Option<RelayHint>,
    cancel: watch::Sender<bool>,
    actor_started: AtomicBool,
    primary: AtomicBool,
    admission: CandidateAdmission,
    metrics: Arc<BrokerMetrics>,
    _connection_permit: OwnedSemaphorePermit,
    _metric: ConnectionMetricGuard,
}

impl fmt::Debug for Candidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Candidate")
            .field("key", &self.key)
            .field("remote", &self.remote)
            .field("side", &self.side)
            .field("inbound_authorization", &self.inbound_authorization)
            .field("verified_relay", &self.verified_relay)
            .finish_non_exhaustive()
    }
}

struct BrokerMetrics {
    authenticated: AtomicUsize,
    primary: AtomicUsize,
    streams: AtomicUsize,
    paths: Mutex<BTreeMap<DeviceId, PathKind>>,
    reporter: NetworkReporter,
}

struct ConnectionMetricGuard {
    metrics: Arc<BrokerMetrics>,
}

struct StreamMetricGuard {
    metrics: Arc<BrokerMetrics>,
    peer_streams: Arc<AtomicUsize>,
}

/// RAII proof that at least one consumer still wants connectivity to a peer.
pub struct ConnectionDemand {
    broker: ConnectionBroker,
    slot: Arc<PeerSlot>,
    transient_routes: Vec<RelayHint>,
    released: bool,
}

/// Address-free selected-path status for one currently promoted connection.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SelectedPathObservation {
    pub(crate) path: PathKind,
    pub(crate) rtt_ms: Option<u32>,
}

/// Address-free observations bound to the exact candidate that opened a
/// service stream. Keeping this handle with the stream prevents a later
/// primary replacement from changing the path reported for an already-open
/// epoch.
#[cfg(unix)]
#[derive(Clone)]
pub(crate) struct SelectedCandidateObserver {
    candidate: Arc<Candidate>,
}

#[cfg(unix)]
impl SelectedCandidateObserver {
    /// Observes only the selected path class and its current RTT estimate.
    /// Remote addresses and relay URLs never cross this boundary.
    pub(crate) fn selected_path_observation(&self) -> SelectedPathObservation {
        self.candidate
            .connection
            .paths()
            .iter()
            .find(|path| path.is_selected())
            .map_or_else(SelectedPathObservation::default, |path| {
                let (kind, _) = classify_transport_addr(path.remote_addr());
                SelectedPathObservation {
                    path: kind,
                    rtt_ms: Some(round_rtt_millis(path.rtt())),
                }
            })
    }
}

/// Normal-ALPN proof that the remote receiver currently authorizes this host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationConfirmation {
    remote: DeviceId,
    generation: AuthGeneration,
    verified_relay: Option<RelayHint>,
}

impl AuthorizationConfirmation {
    /// Receiver whose normal handshake proved authorization.
    #[must_use]
    pub const fn remote(&self) -> DeviceId {
        self.remote
    }

    /// Current receiver-owned authorization generation.
    #[must_use]
    pub const fn generation(&self) -> AuthGeneration {
        self.generation
    }

    /// Relay selected by the promoted, TLS/application-authenticated candidate.
    /// Direct IP addresses are never represented here.
    #[must_use]
    pub fn verified_relay(&self) -> Option<&RelayHint> {
        self.verified_relay.as_ref()
    }
}

/// Short-lived, fully TLS-authenticated `zterm-pair/2` connection.
///
/// This owner intentionally exposes stream operations rather than the raw
/// Endpoint or normal peer registry. Its admission permit remains held until
/// drop, and network quiesce closes it even if a handshake stream is stalled.
pub struct PairConnection {
    connection: Connection,
    local: DeviceId,
    remote: DeviceId,
    verified_relay: Option<RelayHint>,
    shutdown: watch::Receiver<bool>,
    cancellation_task: Option<JoinHandle<()>>,
    _admission: PairHandshakePermit,
    closed: bool,
}

impl fmt::Debug for PairConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairConnection")
            .field("local", &self.local)
            .field("remote", &self.remote)
            .field("verified_relay", &self.verified_relay)
            .finish_non_exhaustive()
    }
}

impl PairConnection {
    fn from_authenticated(
        connection: Connection,
        local: DeviceId,
        remote: DeviceId,
        verified_relay: Option<RelayHint>,
        admission: PairHandshakePermit,
        shutdown: watch::Receiver<bool>,
    ) -> Result<Self, DaemonError> {
        if *shutdown.borrow() {
            close_connection(&connection, CLOSE_SHUTTING_DOWN, b"transport stopping");
            return Err(cancelled("pairing transport is stopping"));
        }
        connection.set_max_concurrent_bi_streams(VarInt::from_u32(1));
        connection.set_max_concurrent_uni_streams(VarInt::from_u32(0));
        let mut cancellation = shutdown.clone();
        let closing = connection.clone();
        let cancellation_task = tokio::spawn(async move {
            if !*cancellation.borrow() {
                let _ = cancellation.changed().await;
            }
            closing.close(VarInt::from_u32(CLOSE_SHUTTING_DOWN), b"transport stopping");
        });
        Ok(Self {
            connection,
            local,
            remote,
            verified_relay,
            shutdown,
            cancellation_task: Some(cancellation_task),
            _admission: admission,
            closed: false,
        })
    }

    /// Local long-term endpoint identity.
    #[must_use]
    pub const fn local(&self) -> DeviceId {
        self.local
    }

    /// Remote identity authenticated by the completed Iroh TLS handshake.
    #[must_use]
    pub const fn remote(&self) -> DeviceId {
        self.remote
    }

    /// Relay candidate that survived TLS, if the dial used one.
    #[must_use]
    pub fn verified_relay(&self) -> Option<&RelayHint> {
        self.verified_relay.as_ref()
    }

    /// Opens the controller side's sole pairing stream before the same
    /// absolute deadline.
    pub async fn open_bi(
        &self,
        deadline: Instant,
    ) -> Result<(SendStream, RecvStream), DaemonError> {
        timeout_or_pair_shutdown(deadline, self.shutdown.clone(), self.connection.open_bi())
            .await?
            .map_err(|_| transport_unavailable("unable to open pairing stream"))
    }

    /// Accepts the host side's sole pairing stream before the same absolute
    /// deadline.
    pub async fn accept_bi(
        &self,
        deadline: Instant,
    ) -> Result<(SendStream, RecvStream), DaemonError> {
        timeout_or_pair_shutdown(deadline, self.shutdown.clone(), self.connection.accept_bi())
            .await?
            .map_err(|_| transport_unavailable("unable to accept pairing stream"))
    }

    /// Explicitly closes only this short-lived pair connection.
    pub fn close(mut self) {
        self.close_inner();
    }

    fn close_inner(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        if let Some(task) = self.cancellation_task.take() {
            task.abort();
        }
        self.connection.close(
            VarInt::from_u32(CLOSE_PAIR_COMPLETE),
            b"pairing connection closed",
        );
    }
}

impl Drop for PairConnection {
    fn drop(&mut self) {
        self.close_inner();
    }
}

impl fmt::Debug for ConnectionDemand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionDemand")
            .field("remote", &self.slot.remote)
            .finish_non_exhaustive()
    }
}

/// A promoted stream plus the remote receiver generation proved by Welcome.
pub struct AuthenticatedBiStream {
    /// Send half of the Iroh bidirectional stream.
    pub send: SendStream,
    /// Receive half of the Iroh bidirectional stream.
    pub recv: RecvStream,
    remote: DeviceId,
    remote_generation: AuthGeneration,
    candidate: ConnectionCandidateKey,
    #[cfg(unix)]
    candidate_observer: SelectedCandidateObserver,
    purpose: StreamPurpose,
    _stream_permit: OwnedSemaphorePermit,
    _metric: StreamMetricGuard,
}

impl fmt::Debug for AuthenticatedBiStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedBiStream")
            .field("remote", &self.remote)
            .field("remote_generation", &self.remote_generation)
            .field("candidate", &self.candidate)
            .field("purpose", &self.purpose)
            .finish_non_exhaustive()
    }
}

impl AuthenticatedBiStream {
    /// Remote device whose receiver admitted this host.
    #[must_use]
    pub const fn remote(&self) -> DeviceId {
        self.remote
    }

    /// Receiver-side authorization generation returned in ConnectionWelcome.
    #[must_use]
    pub const fn remote_generation(&self) -> AuthGeneration {
        self.remote_generation
    }

    /// Deterministic primary candidate used to open the stream.
    #[must_use]
    pub const fn candidate(&self) -> ConnectionCandidateKey {
        self.candidate
    }

    /// Cloneable observations tied to the candidate that opened this stream.
    #[cfg(unix)]
    pub(crate) fn candidate_observer(&self) -> SelectedCandidateObserver {
        self.candidate_observer.clone()
    }
}

impl<T> Default for CandidateRegistry<T> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            primary: None,
        }
    }
}

impl<T> CandidateRegistry<T> {
    fn register(&mut self, key: ConnectionCandidateKey, value: T) -> Result<(), T> {
        if self.entries.contains_key(&key) {
            return Err(value);
        }
        let previous = self.entries.insert(key, CandidateEntry { value });
        debug_assert!(previous.is_none());
        Ok(())
    }

    fn mark_ready_and_decide(&mut self, key: ConnectionCandidateKey) -> CandidateDecision<T> {
        if !self.entries.contains_key(&key) {
            return CandidateDecision::Missing;
        }

        if self.primary.is_some_and(|primary| primary < key) {
            return self
                .remove(key)
                .map_or(CandidateDecision::Missing, CandidateDecision::Lost);
        }

        if designated_primary(self.entries.keys().copied()) != Some(key) {
            return CandidateDecision::Wait;
        }

        self.primary = Some(key);
        let loser_keys = self
            .entries
            .keys()
            .copied()
            .filter(|candidate| *candidate != key)
            .collect::<Vec<_>>();
        let losers = loser_keys
            .into_iter()
            .filter_map(|loser| self.remove_entry(loser))
            .collect();
        CandidateDecision::Promoted(losers)
    }

    fn primary(&self) -> Option<ConnectionCandidateKey> {
        self.primary
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn get(&self, key: &ConnectionCandidateKey) -> Option<&T> {
        self.entries.get(key).map(|entry| &entry.value)
    }

    fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.values().map(|entry| &entry.value)
    }

    fn remove(&mut self, key: ConnectionCandidateKey) -> Option<T> {
        if self.primary == Some(key) {
            self.primary = None;
        }
        self.remove_entry(key)
    }

    fn remove_entry(&mut self, key: ConnectionCandidateKey) -> Option<T> {
        self.entries.remove(&key).map(|entry| entry.value)
    }

    fn take_all(&mut self) -> Vec<T> {
        self.primary = None;
        std::mem::take(&mut self.entries)
            .into_values()
            .map(|entry| entry.value)
            .collect()
    }
}

impl DemandState {
    fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            transient_routes: Mutex::new(TransientRouteLeases::default()),
        }
    }

    fn acquire(
        &self,
        routes: Vec<RelayHint>,
        maximum_routes: usize,
    ) -> Result<(usize, Vec<RelayHint>), DaemonError> {
        let previous = increment_atomic_previous(&self.count)
            .map_err(|()| resource_exhausted("peer demand reference count exhausted"))?;
        let routes = match mutex_lock(&self.transient_routes).acquire(routes, maximum_routes) {
            Ok(routes) => routes,
            Err(error) => {
                decrement_atomic(&self.count);
                return Err(error);
            }
        };
        Ok((previous, routes))
    }

    fn release(&self, routes: &[RelayHint]) {
        mutex_lock(&self.transient_routes).release(routes);
        decrement_atomic(&self.count);
    }

    fn count(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }

    fn routes(&self) -> Vec<RelayHint> {
        mutex_lock(&self.transient_routes).snapshot()
    }
}

impl TransientRouteLeases {
    fn acquire(
        &mut self,
        routes: Vec<RelayHint>,
        maximum: usize,
    ) -> Result<Vec<RelayHint>, DaemonError> {
        let mut unique = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for route in routes {
            if seen.insert(route.clone()) {
                unique.push(route);
            }
        }

        let additional = unique
            .iter()
            .filter(|route| !self.ordered.iter().any(|(existing, _)| existing == *route))
            .count();
        let combined = self.ordered.len().checked_add(additional).ok_or_else(|| {
            resource_exhausted("concurrent transient route count exhausted its address space")
        })?;
        if combined > maximum {
            return Err(resource_exhausted(
                "concurrent transient route leases exceed the peer route bound",
            ));
        }
        if unique.iter().any(|route| {
            self.ordered
                .iter()
                .find(|(existing, _)| existing == route)
                .is_some_and(|(_, count)| *count == usize::MAX)
        }) {
            return Err(resource_exhausted(
                "transient route reference count exhausted",
            ));
        }

        for route in &unique {
            if let Some((_, count)) = self
                .ordered
                .iter_mut()
                .find(|(existing, _)| existing == route)
            {
                *count = count.checked_add(1).ok_or_else(|| {
                    resource_exhausted("transient route reference count exhausted")
                })?;
            } else {
                self.ordered.push((route.clone(), 1));
            }
        }
        Ok(unique)
    }

    fn release(&mut self, routes: &[RelayHint]) {
        for route in routes {
            if let Some((_, count)) = self
                .ordered
                .iter_mut()
                .find(|(existing, _)| existing == route)
            {
                if let Some(next) = count.checked_sub(1) {
                    *count = next;
                } else {
                    debug_assert!(false, "transient route lease released more than once");
                }
            }
        }
        self.ordered.retain(|(_, count)| *count > 0);
    }

    fn snapshot(&self) -> Vec<RelayHint> {
        self.ordered
            .iter()
            .map(|(route, _)| route.clone())
            .collect()
    }
}

impl BrokerAdmission {
    fn new(limits: TransportLimits) -> Self {
        Self {
            pending_dials: Arc::new(Semaphore::new(limits.max_pending_dials)),
            authenticated_connections: Arc::new(Semaphore::new(limits.max_remote_connections)),
            global_stream_handlers: Arc::new(Semaphore::new(limits.max_stream_handlers_global)),
        }
    }
}

impl PeerAdmission {
    fn new(limits: TransportLimits) -> Self {
        Self {
            open_queue: Arc::new(Semaphore::new(limits.max_open_stream_queue_per_connection)),
        }
    }
}

impl CandidateAdmission {
    fn new(limits: TransportLimits) -> Self {
        Self {
            streams: Arc::new(Semaphore::new(limits.max_bi_streams_per_connection)),
            handlers: Arc::new(Semaphore::new(limits.max_stream_handlers_per_connection)),
        }
    }
}

fn try_admit_service_handler(
    lifecycle: &BrokerLifecycle,
    connection: &Arc<Semaphore>,
    global: &Arc<Semaphore>,
) -> Result<ServiceHandlerPermits, DaemonError> {
    if lifecycle.is_quiescing() {
        return Err(transport_unavailable("network transport is stopping"));
    }
    let connection = Arc::clone(connection)
        .try_acquire_owned()
        .map_err(|_| resource_exhausted("connection handler limit reached"))?;
    let global = Arc::clone(global)
        .try_acquire_owned()
        .map_err(|_| resource_exhausted("global handler limit reached"))?;
    if lifecycle.is_quiescing() {
        return Err(transport_unavailable("network transport is stopping"));
    }
    Ok(ServiceHandlerPermits {
        _connection: connection,
        _global: global,
    })
}

async fn wait_for_service_handlers_until(
    handlers: Arc<Semaphore>,
    maximum: usize,
    deadline: Instant,
) -> Result<(), DaemonError> {
    if handlers.available_permits() == maximum {
        return Ok(());
    }
    let maximum = u32::try_from(maximum)
        .map_err(|_| resource_exhausted("global handler limit exceeds the semaphore boundary"))?;
    let permits = timeout_until(deadline, handlers.acquire_many_owned(maximum))
        .await?
        .map_err(|_| transport_unavailable("global handler admission is unavailable"))?;
    drop(permits);
    Ok(())
}

impl BrokerLifecycle {
    fn is_quiescing(&self) -> bool {
        self.shutting_down.load(Ordering::Acquire)
    }

    fn begin_quiesce(&self) {
        self.shutting_down.store(true, Ordering::Release);
    }
}

impl PeerState {
    fn claim_dial_worker(&mut self) -> bool {
        if self.dial_worker_running {
            false
        } else {
            self.dial_worker_running = true;
            true
        }
    }

    fn begin_demand_cycle(&mut self, previous_demands: usize) {
        if previous_demands == 0 {
            self.terminal_error = None;
        }
    }

    fn close_candidates(&mut self) -> Vec<Arc<Candidate>> {
        self.remote_acceptance = None;
        self.candidates.take_all()
    }
}

impl ConnectionBroker {
    pub(crate) fn with_reporter(
        identity: ConnectionIdentity,
        store: StoreHandle,
        authorization: AuthorizationRegistry,
        limits: TransportLimits,
        reporter: NetworkReporter,
        observer: NetworkObserver,
    ) -> Result<Self, DaemonError> {
        validate_limits(limits)?;
        let resolver = RouteResolver::new(store.clone(), limits)?;
        let pairing = PairHandshakeAdmission::new(limits)?;
        let (endpoint, _) = watch::channel(None);
        let metrics = Arc::new(BrokerMetrics {
            authenticated: AtomicUsize::new(0),
            primary: AtomicUsize::new(0),
            streams: AtomicUsize::new(0),
            paths: Mutex::new(BTreeMap::new()),
            reporter: reporter.clone(),
        });
        Ok(Self {
            inner: Arc::new(BrokerInner {
                identity,
                store,
                authorization,
                resolver,
                limits,
                endpoint,
                peers: Mutex::new(BTreeMap::new()),
                admission: BrokerAdmission::new(limits),
                metrics,
                observer,
                lifecycle: BrokerLifecycle::default(),
                pairing,
                #[cfg(unix)]
                service_handler: RemoteServiceHandlerSlot::default(),
                test_routes: Mutex::new(BTreeMap::new()),
            }),
        })
    }

    /// Shared pair admission used by the inbound ALPN router and outbound
    /// transient dial path.
    pub(crate) fn pair_handshake_admission(&self) -> PairHandshakeAdmission {
        self.inner.pairing.clone()
    }

    /// Installs the single inbound normal-service owner before Endpoint spawn.
    #[cfg(unix)]
    pub(crate) fn install_remote_service_handler(
        &self,
        handler: Arc<dyn RemoteServiceHandler>,
    ) -> Result<(), DaemonError> {
        if self.inner.lifecycle.is_quiescing() {
            return Err(transport_unavailable("network transport is stopping"));
        }
        self.inner.service_handler.install(handler)
    }

    /// Creates a broker around a task-private already-bound Endpoint.
    ///
    /// Production composition uses the endpoint supervisor. This constructor is
    /// intentionally reserved for deterministic real-Iroh integration tests.
    #[doc(hidden)]
    pub fn for_test(
        endpoint: Endpoint,
        identity: ConnectionIdentity,
        store: StoreHandle,
        authorization: AuthorizationRegistry,
        limits: TransportLimits,
    ) -> Result<Self, DaemonError> {
        if endpoint.id().as_bytes() != identity.device_id.as_bytes() {
            return Err(DaemonError::new(
                DomainErrorKind::IdentityStateMismatch,
                "test Endpoint identity does not match broker identity",
            ));
        }
        let (reporter, observer) = NetworkReporter::initializing(identity.device_id);
        reporter.update(|observation| {
            observation.state = crate::network::NetworkState::Bound;
            observation.endpoint_bound = true;
            observation.bind_attempts = 1;
            observation.publish = crate::network::AddressServiceState::Configured;
            observation.lookup = crate::network::AddressServiceState::Configured;
        });
        let broker =
            Self::with_reporter(identity, store, authorization, limits, reporter, observer)?;
        broker.inner.endpoint.send_replace(Some(endpoint));
        Ok(broker)
    }

    /// Adds one explicit address for a task-private real-Iroh dial fixture.
    ///
    /// The override is never persisted, merged into address lookup, or exposed
    /// by status. Production code has no call site for this hook.
    #[doc(hidden)]
    pub fn set_test_route(
        &self,
        remote: DeviceId,
        address: EndpointAddr,
    ) -> Result<(), DaemonError> {
        if address.id != endpoint_id_from_device(remote)? {
            return Err(DaemonError::new(
                DomainErrorKind::AddressUnavailable,
                "test route endpoint ID does not match its device key",
            ));
        }
        mutex_lock(&self.inner.test_routes).insert(remote, address);
        Ok(())
    }

    /// Returns the shared passive network observation.
    #[must_use]
    pub fn observe(&self) -> NetworkObserver {
        self.inner.observer.clone()
    }

    /// Returns redacted live state for one peer.
    pub async fn peer_observation(&self, remote: DeviceId) -> PeerConnectionObservation {
        let Some(slot) = mutex_lock(&self.inner.peers).get(&remote).cloned() else {
            return PeerConnectionObservation {
                primary: None,
                candidate_count: 0,
                demand_count: 0,
                active_stream_count: 0,
                remote_acceptance_generation: None,
                path: PathKind::Unknown,
            };
        };
        let state = slot.state.lock().await;
        PeerConnectionObservation {
            primary: state.candidates.primary(),
            candidate_count: state.candidates.len(),
            demand_count: slot.demand.count(),
            active_stream_count: bounded_u32(slot.active_streams.load(Ordering::Acquire)),
            remote_acceptance_generation: state.remote_acceptance,
            path: self.inner.metrics.path(remote),
        }
    }

    /// Acquires one demand for a durable outbound known device.
    pub async fn demand(
        &self,
        remote: DeviceId,
        deadline: Instant,
    ) -> Result<ConnectionDemand, DaemonError> {
        if self.inner.lifecycle.is_quiescing() {
            return Err(transport_unavailable("network transport is stopping"));
        }
        let known = self
            .inner
            .store
            .run_blocking_until(deadline, move |store, deadline| {
                store.known_device(remote, deadline)
            })
            .await?;
        if known.is_none() {
            return Err(DaemonError::new(
                DomainErrorKind::DeviceNotFound,
                "target is not an outbound known device",
            ));
        }
        self.acquire_demand(remote, Vec::new()).await
    }

    /// Acquires one short-lived demand using validated pairing-ticket routes.
    pub async fn demand_transient(
        &self,
        remote: DeviceId,
        routes: Vec<RelayHint>,
    ) -> Result<ConnectionDemand, DaemonError> {
        if self.inner.lifecycle.is_quiescing() {
            return Err(transport_unavailable("network transport is stopping"));
        }
        if routes.is_empty() || routes.len() > self.inner.limits.max_relay_hints {
            return Err(DaemonError::new(
                DomainErrorKind::AddressUnavailable,
                "transient target requires a bounded relay route",
            ));
        }
        self.acquire_demand(remote, routes).await
    }

    /// Establishes one short-lived pair-ALPN connection without touching the
    /// normal peer registry, primary metrics, route cache, or profile.
    pub(crate) async fn connect_pair_transient(
        &self,
        remote: DeviceId,
        routes: Vec<RelayHint>,
        deadline: Instant,
    ) -> Result<PairConnection, DaemonError> {
        if self.inner.lifecycle.is_quiescing() {
            return Err(cancelled("pairing transport is stopping"));
        }
        if remote == self.inner.identity.device_id {
            return Err(DaemonError::new(
                DomainErrorKind::Unauthorized,
                "a device cannot pair with its own endpoint identity",
            ));
        }
        if routes.is_empty() || routes.len() > self.inner.limits.max_relay_hints {
            return Err(DaemonError::new(
                DomainErrorKind::AddressUnavailable,
                "pairing target requires a bounded relay route",
            ));
        }

        let admission = self.inner.pairing.acquire(remote, deadline).await?;
        let _dial_permit = timeout_or_pair_shutdown(
            deadline,
            self.inner.pairing.subscribe(),
            self.inner.admission.pending_dials.clone().acquire_owned(),
        )
        .await?
        .map_err(|_| transport_unavailable("global outbound dial admission is unavailable"))?;
        let endpoint = self
            .inner
            .endpoint
            .borrow()
            .clone()
            .ok_or_else(|| transport_unavailable("Iroh Endpoint is not bound"))?;
        if endpoint.id().as_bytes() != self.inner.identity.device_id.as_bytes() {
            return Err(DaemonError::new(
                DomainErrorKind::IdentityStateMismatch,
                "bound Endpoint does not own the broker identity",
            ));
        }

        let dial_routes = timeout_or_pair_shutdown(
            deadline,
            self.inner.pairing.subscribe(),
            self.dial_routes(&endpoint, remote, &routes, deadline),
        )
        .await??;
        let mut last_error = None;
        for route in dial_routes {
            if Instant::now() >= deadline {
                return Err(deadline_exceeded("pairing dial deadline elapsed"));
            }
            let attempt_deadline = deadline.min(
                Instant::now()
                    .checked_add(self.inner.limits.connect_attempt_budget)
                    .unwrap_or(deadline),
            );
            let result = timeout_or_pair_shutdown(
                attempt_deadline,
                self.inner.pairing.subscribe(),
                endpoint.connect(route.address, ZTERM_PAIR_ALPN),
            )
            .await;
            let connection = match result {
                Ok(Ok(connection)) => connection,
                Ok(Err(_)) => {
                    last_error = Some(transport_unavailable("pairing connect attempt failed"));
                    continue;
                }
                Err(error) if should_try_next_pair_route(&error, deadline) => {
                    last_error = Some(error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            if let Err(error) = validate_pair_tls_connection(&connection, remote) {
                close_connection(&connection, CLOSE_INCOMPATIBLE, b"identity mismatch");
                return Err(error);
            }
            return PairConnection::from_authenticated(
                connection,
                self.inner.identity.device_id,
                remote,
                route.verified_relay,
                admission,
                self.inner.pairing.subscribe(),
            );
        }

        Err(last_error.unwrap_or_else(|| {
            DaemonError::new(
                DomainErrorKind::AddressUnavailable,
                "no route candidate established a pairing connection",
            )
        }))
    }

    /// Wraps one fully awaited inbound pair connection without registering a
    /// normal candidate or changing transport metrics.
    pub(crate) fn pair_from_incoming(
        &self,
        connection: Connection,
        admission: PairHandshakePermit,
    ) -> Result<PairConnection, DaemonError> {
        let remote = device_from_endpoint_id(connection.remote_id());
        if let Err(error) = validate_pair_tls_connection(&connection, remote) {
            close_connection(&connection, CLOSE_INCOMPATIBLE, b"identity mismatch");
            return Err(error);
        }
        if remote == self.inner.identity.device_id {
            close_connection(&connection, CLOSE_UNAUTHORIZED, b"not authorized");
            return Err(DaemonError::new(
                DomainErrorKind::Unauthorized,
                "a device cannot pair with its own endpoint identity",
            ));
        }
        let verified_relay = selected_relay(&connection);
        PairConnection::from_authenticated(
            connection,
            self.inner.identity.device_id,
            remote,
            verified_relay,
            admission,
            self.inner.pairing.subscribe(),
        )
    }

    async fn acquire_demand(
        &self,
        remote: DeviceId,
        transient_routes: Vec<RelayHint>,
    ) -> Result<ConnectionDemand, DaemonError> {
        if remote == self.inner.identity.device_id {
            return Err(DaemonError::new(
                DomainErrorKind::TransportUnavailable,
                "the local device cannot be reached through its own broker",
            ));
        }
        let slot = self.peer_slot(remote);
        let (previous_demands, transient_routes) = slot
            .demand
            .acquire(transient_routes, self.inner.limits.max_relay_hints)?;
        // Construct the RAII owner before the first await. Cancellation of this
        // future therefore cannot leak the demand count or ticket routes.
        let demand = ConnectionDemand {
            broker: self.clone(),
            slot: Arc::clone(&slot),
            transient_routes,
            released: false,
        };
        slot.state.lock().await.begin_demand_cycle(previous_demands);
        self.ensure_dial_worker(Arc::clone(&slot)).await?;
        slot.changed.notify_waiters();
        Ok(demand)
    }

    fn peer_slot(&self, remote: DeviceId) -> Arc<PeerSlot> {
        let mut peers = mutex_lock(&self.inner.peers);
        peers
            .entry(remote)
            .or_insert_with(|| {
                Arc::new(PeerSlot {
                    remote,
                    state: AsyncMutex::new(PeerState::default()),
                    changed: Notify::new(),
                    demand: DemandState::new(),
                    admission: PeerAdmission::new(self.inner.limits),
                    active_streams: Arc::new(AtomicUsize::new(0)),
                })
            })
            .clone()
    }

    async fn ensure_dial_worker(&self, slot: Arc<PeerSlot>) -> Result<(), DaemonError> {
        if self.inner.lifecycle.is_quiescing() {
            return Err(transport_unavailable("network transport is stopping"));
        }
        let mut state = slot.state.lock().await;
        if state.claim_dial_worker() {
            let broker = self.clone();
            drop(state);
            tokio::spawn(async move {
                broker.run_dial_worker(slot).await;
            });
        }
        Ok(())
    }

    /// Registers a fully TLS-authenticated inbound normal connection.
    ///
    /// Authorization is checked before this function accepts or reads the
    /// unique Hello stream.
    pub async fn accept_normal(&self, connection: Connection) -> Result<(), DaemonError> {
        if connection.alpn() != ZTERM_ALPN {
            close_connection(&connection, CLOSE_INCOMPATIBLE, b"incompatible protocol");
            return Err(DaemonError::new(
                DomainErrorKind::WireMajorMismatch,
                "incoming connection negotiated an unexpected ALPN",
            ));
        }
        configure_normal_connection_limits(&connection, self.inner.limits);
        let remote = device_from_endpoint_id(connection.remote_id());
        let admission = match admit_inbound_before_payload(&self.inner.authorization, remote) {
            Ok(admission) => admission,
            Err(error) => {
                close_connection(&connection, CLOSE_UNAUTHORIZED, b"not authorized");
                return Err(error);
            }
        };
        let connection_permit = self
            .inner
            .admission
            .authenticated_connections
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                close_connection(&connection, CLOSE_OVERLOADED, b"transport overloaded");
                resource_exhausted("authenticated connection limit reached")
            })?;
        self.run_inbound_handshake(connection, remote, admission, connection_permit)
            .await
    }

    /// Replaces the current Endpoint after a supervisor bind succeeds.
    pub(crate) async fn attach_endpoint(&self, endpoint: Endpoint) -> Result<(), DaemonError> {
        if self.inner.lifecycle.is_quiescing() {
            endpoint.close().await;
            return Err(transport_unavailable("network transport is stopping"));
        }
        if endpoint.id().as_bytes() != self.inner.identity.device_id.as_bytes() {
            endpoint.close().await;
            return Err(DaemonError::new(
                DomainErrorKind::IdentityStateMismatch,
                "bound Endpoint identity changed across retry",
            ));
        }
        self.inner.endpoint.send_replace(Some(endpoint));
        self.clear_retryable_errors().await;
        self.wake_all_peers();
        Ok(())
    }

    /// Removes a lost Endpoint and closes only transport state.
    pub(crate) async fn detach_endpoint(&self) {
        self.inner.endpoint.send_replace(None);
        self.close_all(ConnectionCloseReason::EndpointReset).await;
        self.wake_all_peers();
    }

    /// Permanently refuses new work and closes every current connection.
    pub async fn quiesce(&self) {
        let deadline = Instant::now() + self.inner.limits.first_frame_deadline;
        let _ = self.quiesce_until(deadline).await;
    }

    /// Permanently refuses new work and reclaims every admitted service
    /// handler before the caller closes the owning Endpoint.
    pub(crate) async fn quiesce_until(&self, deadline: Instant) -> Result<(), DaemonError> {
        self.inner.lifecycle.begin_quiesce();
        self.inner.pairing.begin_quiesce();
        self.inner.endpoint.send_replace(None);
        self.close_all(ConnectionCloseReason::ShuttingDown).await;
        self.wake_all_peers();
        wait_for_service_handlers_until(
            Arc::clone(&self.inner.admission.global_stream_handlers),
            self.inner.limits.max_stream_handlers_global,
            deadline,
        )
        .await
    }

    /// Closes all current candidates for one endpoint without touching Session state.
    pub async fn close_remote(&self, remote: DeviceId, reason: ConnectionCloseReason) {
        let Some(slot) = mutex_lock(&self.inner.peers).get(&remote).cloned() else {
            return;
        };
        let candidates = {
            let mut state = slot.state.lock().await;
            state.close_candidates()
        };
        for candidate in candidates {
            candidate.set_primary(false, reason.code());
            candidate.cancel(reason);
        }
        slot.changed.notify_waiters();
    }

    async fn close_all(&self, reason: ConnectionCloseReason) {
        let slots = mutex_lock(&self.inner.peers)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for slot in slots {
            self.close_remote(slot.remote, reason).await;
        }
    }

    fn wake_all_peers(&self) {
        for slot in mutex_lock(&self.inner.peers).values() {
            slot.changed.notify_waiters();
        }
    }

    async fn clear_retryable_errors(&self) {
        let slots = mutex_lock(&self.inner.peers)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for slot in slots {
            let mut state = slot.state.lock().await;
            if state
                .terminal_error
                .as_ref()
                .is_some_and(|error| error.kind() == DomainErrorKind::TransportUnavailable)
            {
                state.terminal_error = None;
            }
        }
    }
}

impl ConnectionDemand {
    fn ensure_active(&self) -> Result<(), DaemonError> {
        if self.released {
            Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                "connection demand has already been released",
            ))
        } else {
            Ok(())
        }
    }

    async fn wait_for_confirmed_primary(
        &self,
        deadline: Instant,
    ) -> Result<(Arc<Candidate>, AuthGeneration), DaemonError> {
        self.ensure_active()?;
        loop {
            if Instant::now() >= deadline {
                return Err(deadline_exceeded(
                    "timed out waiting for a primary connection",
                ));
            }
            if self.broker.inner.lifecycle.is_quiescing() {
                return Err(transport_unavailable("network transport is stopping"));
            }

            let selection = {
                let state = self.slot.state.lock().await;
                if let Some(error) = &state.terminal_error
                    && !is_retryable(error.kind())
                {
                    return Err(error.clone());
                }
                match (state.candidates.primary(), state.remote_acceptance) {
                    (Some(key), Some(generation)) => state
                        .candidates
                        .get(&key)
                        .map(|candidate| (Arc::clone(candidate), generation)),
                    _ => None,
                }
            };

            if let Some(selection) = selection {
                return Ok(selection);
            }
            self.broker
                .ensure_dial_worker(Arc::clone(&self.slot))
                .await?;
            wait_for_notify(&self.slot.changed, deadline).await?;
        }
    }

    /// Waits for normal Hello/Welcome authorization proof without opening an
    /// application stream.
    pub async fn confirm_authorization(
        &self,
        deadline: Instant,
    ) -> Result<AuthorizationConfirmation, DaemonError> {
        let (candidate, generation) = self.wait_for_confirmed_primary(deadline).await?;
        Ok(AuthorizationConfirmation {
            remote: self.slot.remote,
            generation,
            verified_relay: candidate.verified_relay.clone(),
        })
    }

    /// Opens a business stream only after deterministic candidate promotion.
    pub async fn open_bi(
        &self,
        purpose: StreamPurpose,
        deadline: Instant,
    ) -> Result<AuthenticatedBiStream, DaemonError> {
        self.ensure_active()?;
        let _queue = self
            .slot
            .admission
            .open_queue
            .clone()
            .try_acquire_owned()
            .map_err(|_| resource_exhausted("peer open-stream queue is full"))?;

        loop {
            let (candidate, generation) = self.wait_for_confirmed_primary(deadline).await?;
            let stream_permit = acquire_until(
                Arc::clone(&candidate.admission.streams),
                deadline,
                "connection stream limit reached",
            )
            .await?;

            let still_primary = {
                let state = self.slot.state.lock().await;
                state.candidates.primary() == Some(candidate.key)
                    && state.remote_acceptance == Some(generation)
            };
            if !still_primary {
                drop(stream_permit);
                continue;
            }

            match timeout_until(deadline, candidate.connection.open_bi()).await {
                Ok(Ok((send, recv))) => {
                    let metric = StreamMetricGuard::new(
                        Arc::clone(&self.broker.inner.metrics),
                        Arc::clone(&self.slot.active_streams),
                    )?;
                    return Ok(AuthenticatedBiStream {
                        send,
                        recv,
                        remote: self.slot.remote,
                        remote_generation: generation,
                        candidate: candidate.key,
                        #[cfg(unix)]
                        candidate_observer: SelectedCandidateObserver {
                            candidate: Arc::clone(&candidate),
                        },
                        purpose,
                        _stream_permit: stream_permit,
                        _metric: metric,
                    });
                }
                Ok(Err(_)) => {
                    self.broker
                        .candidate_closed(&self.slot, candidate.key)
                        .await;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for ConnectionDemand {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        self.slot.demand.release(&self.transient_routes);
        self.slot.changed.notify_waiters();
    }
}

impl ConnectionBroker {
    async fn run_dial_worker(&self, slot: Arc<PeerSlot>) {
        'restart: loop {
            let mut retry = 0_u32;
            loop {
                if self.inner.lifecycle.is_quiescing() || slot.demand.count() == 0 {
                    break;
                }
                let action = {
                    let state = slot.state.lock().await;
                    if let Some(error) = &state.terminal_error
                        && !is_retryable(error.kind())
                    {
                        break;
                    }
                    peer_needs_outbound_attempt(
                        &state.candidates,
                        state.remote_acceptance,
                        |candidate| candidate.side == CandidateSide::Outbound,
                    )
                };

                if !action {
                    if wait_for_notify_optional(&slot.changed, Duration::from_secs(30))
                        .await
                        .is_err()
                    {
                        continue;
                    }
                    retry = 0;
                    continue;
                }

                let result = self.dial_once(Arc::clone(&slot)).await;
                match result {
                    Ok(()) => retry = 0,
                    Err(error) if is_retryable(error.kind()) => {
                        {
                            let mut state = slot.state.lock().await;
                            state.terminal_error = Some(error);
                        }
                        let delay = retry_delay(retry, slot.remote);
                        retry = retry.saturating_add(1);
                        tokio::select! {
                            () = tokio::time::sleep(delay) => {}
                            () = slot.changed.notified() => {}
                        }
                        let mut state = slot.state.lock().await;
                        if state
                            .terminal_error
                            .as_ref()
                            .is_some_and(|stored| is_retryable(stored.kind()))
                        {
                            state.terminal_error = None;
                        }
                    }
                    Err(error) => {
                        let mut state = slot.state.lock().await;
                        state.terminal_error = Some(error);
                        slot.changed.notify_waiters();
                        break;
                    }
                }
            }

            let mut state = slot.state.lock().await;
            state.dial_worker_running = false;
            let should_restart = !self.inner.lifecycle.is_quiescing()
                && slot.demand.count() > 0
                && state.terminal_error.is_none()
                && (state.candidates.primary().is_none() || state.remote_acceptance.is_none());
            if should_restart {
                state.dial_worker_running = true;
                drop(state);
                continue 'restart;
            }
            break;
        }
    }

    async fn dial_once(&self, slot: Arc<PeerSlot>) -> Result<(), DaemonError> {
        let _dial_permit = self
            .inner
            .admission
            .pending_dials
            .clone()
            .try_acquire_owned()
            .map_err(|_| resource_exhausted("global outbound dial limit reached"))?;
        let endpoint = self
            .inner
            .endpoint
            .borrow()
            .clone()
            .ok_or_else(|| transport_unavailable("Iroh Endpoint is not bound"))?;
        if endpoint.id().as_bytes() != self.inner.identity.device_id.as_bytes() {
            return Err(DaemonError::new(
                DomainErrorKind::IdentityStateMismatch,
                "bound Endpoint does not own the broker identity",
            ));
        }

        let transient_routes = slot.demand.routes();
        let deadline = Instant::now()
            + self.inner.limits.address_lookup_budget
            + self.inner.limits.connect_attempt_budget
            + self.inner.limits.first_frame_deadline;
        let routes = self
            .dial_routes(&endpoint, slot.remote, &transient_routes, deadline)
            .await?;
        let attempt = random_attempt_id()?;
        let key = ConnectionCandidateKey::new(self.inner.identity.device_id, attempt);
        let mut last_error = None;

        for route in routes {
            if self.inner.lifecycle.is_quiescing() || slot.demand.count() == 0 {
                return Err(DaemonError::new(
                    DomainErrorKind::Cancelled,
                    "outbound demand ended before dial",
                ));
            }
            let connect_deadline = Instant::now() + self.inner.limits.connect_attempt_budget;
            let connection = match timeout_until(
                connect_deadline,
                endpoint.connect(route.address.clone(), ZTERM_ALPN),
            )
            .await
            {
                Ok(Ok(connection)) => connection,
                Ok(Err(_)) => {
                    last_error = Some(transport_unavailable("Iroh connect attempt failed"));
                    continue;
                }
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            configure_normal_connection_limits(&connection, self.inner.limits);
            if device_from_endpoint_id(connection.remote_id()) != slot.remote {
                close_connection(&connection, CLOSE_INCOMPATIBLE, b"identity mismatch");
                return Err(DaemonError::new(
                    DomainErrorKind::Unauthorized,
                    "connected Iroh identity did not match the target device",
                ));
            }
            let connection_permit = match self
                .inner
                .admission
                .authenticated_connections
                .clone()
                .try_acquire_owned()
            {
                Ok(permit) => permit,
                Err(_) => {
                    close_connection(&connection, CLOSE_OVERLOADED, b"transport overloaded");
                    return Err(resource_exhausted("authenticated connection limit reached"));
                }
            };

            let result = self
                .run_outbound_handshake(
                    Arc::clone(&slot),
                    connection,
                    key,
                    route.verified_relay,
                    connection_permit,
                )
                .await;
            match result {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == DomainErrorKind::TransportUnavailable => {
                    let state = slot.state.lock().await;
                    if state.remote_acceptance.is_some() && !state.candidates.is_empty() {
                        // This outbound handshake already proved remote
                        // acceptance and only lost duplicate arbitration. A
                        // remaining peer candidate owns progress; dialing the
                        // next route here would create a self-exciting race.
                        return Ok(());
                    }
                    drop(state);
                    last_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }

        Err(last_error.unwrap_or_else(|| {
            DaemonError::new(
                DomainErrorKind::AddressUnavailable,
                "no route candidate established a connection",
            )
        }))
    }

    async fn dial_routes(
        &self,
        endpoint: &Endpoint,
        remote: DeviceId,
        transient: &[RelayHint],
        deadline: Instant,
    ) -> Result<Vec<DialRoute>, DaemonError> {
        if let Some(address) = mutex_lock(&self.inner.test_routes).get(&remote).cloned() {
            return Ok(vec![DialRoute {
                address,
                verified_relay: None,
            }]);
        }
        self.inner
            .resolver
            .candidates(endpoint, remote, transient, deadline)
            .await
            .map(|candidates| {
                candidates
                    .into_iter()
                    .map(|candidate| DialRoute {
                        address: candidate.endpoint_addr().clone(),
                        verified_relay: Some(candidate.relay_hint().clone()),
                    })
                    .collect()
            })
    }

    async fn run_outbound_handshake(
        &self,
        slot: Arc<PeerSlot>,
        connection: Connection,
        key: ConnectionCandidateKey,
        verified_relay: Option<RelayHint>,
        connection_permit: OwnedSemaphorePermit,
    ) -> Result<(), DaemonError> {
        let candidate = Candidate::new(
            key,
            slot.remote,
            connection,
            CandidateSide::Outbound,
            None,
            verified_relay,
            connection_permit,
            Arc::clone(&self.inner.metrics),
            self.inner.limits,
        )?;
        self.register_candidate(&slot, Arc::clone(&candidate))
            .await?;
        let deadline = Instant::now() + self.inner.limits.first_frame_deadline;
        let result = async {
            let (mut send, mut recv) = timeout_until(deadline, candidate.connection.open_bi())
                .await?
                .map_err(|_| transport_unavailable("unable to open Hello stream"))?;
            let hello = ConnectionHello::new(
                zterm_proto::WIRE_MAJOR,
                zterm_proto::WIRE_MAJOR,
                self.inner.identity.capabilities,
                key.attempt,
                self.inner.identity.display_name.as_str(),
                self.inner.identity.build.clone(),
                self.inner.identity.platform.clone(),
            )
            .map_err(|error| {
                DaemonError::new(
                    DomainErrorKind::IdentityInvalid,
                    format!("local Hello became invalid: {error}"),
                )
            })?;
            write_handshake_message(
                &mut send,
                WireKind::ConnectionHello,
                &v2::ConnectionHello::from(&hello),
                deadline,
            )
            .await?;
            send.finish()
                .map_err(|_| transport_unavailable("unable to finish Hello stream"))?;
            let frame = read_handshake_frame(
                &mut recv,
                self.inner.limits.max_pair_hello_frame_bytes,
                deadline,
            )
            .await
            .map_err(|error| {
                if matches!(
                    error.kind(),
                    DomainErrorKind::MalformedFrame
                        | DomainErrorKind::FrameTooLarge
                        | DomainErrorKind::ControlPayloadTooLarge
                ) {
                    error
                } else {
                    DaemonError::new(
                        DomainErrorKind::Unauthorized,
                        "remote did not complete an authorized normal handshake",
                    )
                }
            })?;
            let wire: v2::ConnectionWelcome = frame
                .decode_message(WireKind::ConnectionWelcome)
                .map_err(protocol_error)?;
            let welcome = ConnectionWelcome::try_from(wire).map_err(|error| {
                DaemonError::new(
                    DomainErrorKind::WireMajorMismatch,
                    format!("invalid ConnectionWelcome: {error}"),
                )
            })?;
            if welcome.wire_major() != zterm_proto::WIRE_MAJOR {
                return Err(DaemonError::new(
                    DomainErrorKind::WireMajorMismatch,
                    "remote selected an incompatible wire major",
                ));
            }
            {
                let mut state = slot.state.lock().await;
                state.remote_acceptance = Some(welcome.accepted_authorization_generation());
                state.terminal_error = None;
            }
            slot.changed.notify_waiters();
            Ok(())
        }
        .await;

        if let Err(error) = result {
            self.remove_candidate(&slot, key, ConnectionCloseReason::EndpointReset)
                .await;
            return Err(error);
        }

        self.persist_verified_handshake(&candidate);
        self.promote_candidate(slot, candidate).await
    }

    async fn run_inbound_handshake(
        &self,
        connection: Connection,
        remote: DeviceId,
        admission: crate::authorization::Admission,
        connection_permit: OwnedSemaphorePermit,
    ) -> Result<(), DaemonError> {
        let deadline = Instant::now() + self.inner.limits.first_frame_deadline;
        let (mut send, mut recv) = timeout_until(deadline, connection.accept_bi())
            .await?
            .map_err(|_| {
                close_connection(&connection, CLOSE_INCOMPATIBLE, b"missing handshake");
                transport_unavailable("incoming connection did not open Hello stream")
            })?;
        let frame = read_handshake_frame(
            &mut recv,
            self.inner.limits.max_pair_hello_frame_bytes,
            deadline,
        )
        .await
        .inspect_err(|_| {
            close_connection(&connection, CLOSE_INCOMPATIBLE, b"invalid handshake");
        })?;
        let wire: v2::ConnectionHello = frame
            .decode_message(WireKind::ConnectionHello)
            .map_err(protocol_error)?;
        let hello = ConnectionHello::try_from(wire).map_err(|error| {
            DaemonError::new(
                DomainErrorKind::WireMajorMismatch,
                format!("invalid ConnectionHello: {error}"),
            )
        })?;
        if hello.min_wire_major() > zterm_proto::WIRE_MAJOR
            || hello.max_wire_major() < zterm_proto::WIRE_MAJOR
        {
            close_connection(&connection, CLOSE_INCOMPATIBLE, b"incompatible protocol");
            return Err(DaemonError::new(
                DomainErrorKind::WireMajorMismatch,
                "normal connection has no common wire major",
            ));
        }
        let key = ConnectionCandidateKey::new(remote, hello.attempt_id());
        let verified_relay = selected_relay(&connection);
        let candidate = Candidate::new(
            key,
            remote,
            connection,
            CandidateSide::Inbound,
            Some(admission.snapshot),
            verified_relay,
            connection_permit,
            Arc::clone(&self.inner.metrics),
            self.inner.limits,
        )?;
        let slot = self.peer_slot(remote);
        self.register_candidate(&slot, Arc::clone(&candidate))
            .await?;

        // Registration precedes this exact-generation recheck so revoke has
        // no gap: if it published before registration this check rejects, and
        // if it publishes after registration close_remote observes the
        // provisional candidate.
        let admission = match recheck_inbound_admission(
            &self.inner.authorization,
            remote,
            admission.snapshot,
        ) {
            Ok(admission) => admission,
            Err(error) => {
                self.remove_candidate(&slot, key, ConnectionCloseReason::Unauthorized)
                    .await;
                return Err(error);
            }
        };

        let welcome = ConnectionWelcome::new(
            zterm_proto::WIRE_MAJOR,
            self.inner.identity.capabilities,
            self.inner.identity.display_name.as_str(),
            self.inner.identity.build.clone(),
            self.inner.identity.platform.clone(),
            admission.snapshot.generation,
        )
        .map_err(|error| {
            DaemonError::new(
                DomainErrorKind::IdentityInvalid,
                format!("local Welcome became invalid: {error}"),
            )
        })?;
        if let Err(error) = write_handshake_message(
            &mut send,
            WireKind::ConnectionWelcome,
            &v2::ConnectionWelcome::from(&welcome),
            deadline,
        )
        .await
        {
            self.remove_candidate(&slot, key, ConnectionCloseReason::EndpointReset)
                .await;
            return Err(error);
        }
        if send.finish().is_err() {
            self.remove_candidate(&slot, key, ConnectionCloseReason::EndpointReset)
                .await;
            return Err(transport_unavailable("unable to finish Welcome stream"));
        }
        self.persist_verified_handshake(&candidate);
        self.promote_candidate(slot, candidate).await
    }
}

struct DialRoute {
    address: EndpointAddr,
    verified_relay: Option<RelayHint>,
}

impl ConnectionBroker {
    async fn register_candidate(
        &self,
        slot: &Arc<PeerSlot>,
        candidate: Arc<Candidate>,
    ) -> Result<(), DaemonError> {
        if self.inner.lifecycle.is_quiescing() {
            candidate.cancel(ConnectionCloseReason::ShuttingDown);
            return Err(transport_unavailable("network transport is stopping"));
        }
        let mut state = slot.state.lock().await;
        if let Err(duplicate) = state.candidates.register(candidate.key, candidate) {
            duplicate.cancel(ConnectionCloseReason::Duplicate);
            return Err(transport_unavailable(
                "duplicate candidate key was registered",
            ));
        }
        drop(state);
        slot.changed.notify_waiters();
        Ok(())
    }

    async fn promote_candidate(
        &self,
        slot: Arc<PeerSlot>,
        candidate: Arc<Candidate>,
    ) -> Result<(), DaemonError> {
        let deadline = Instant::now() + self.inner.limits.connect_attempt_budget;
        loop {
            let outcome = {
                let mut state = slot.state.lock().await;
                state.candidates.mark_ready_and_decide(candidate.key)
            };

            match outcome {
                CandidateDecision::Promoted(losers) => {
                    candidate.set_primary(true, "promoted");
                    for loser in losers {
                        loser.set_primary(false, "duplicate");
                        loser.cancel(ConnectionCloseReason::Duplicate);
                    }
                    slot.changed.notify_waiters();
                    self.start_connection_actor(slot, candidate);
                    return Ok(());
                }
                CandidateDecision::Lost(loser) => {
                    loser.cancel(ConnectionCloseReason::Duplicate);
                    slot.changed.notify_waiters();
                    return Err(transport_unavailable(
                        "connection candidate lost duplicate arbitration",
                    ));
                }
                CandidateDecision::Wait => {
                    if let Err(error) = wait_for_notify(&slot.changed, deadline).await {
                        // A lower provisional candidate must never leave this
                        // ready connection retained forever if arbitration
                        // cannot finish before its bounded deadline.
                        self.remove_candidate(
                            &slot,
                            candidate.key,
                            ConnectionCloseReason::EndpointReset,
                        )
                        .await;
                        return Err(error);
                    }
                }
                CandidateDecision::Missing => {
                    return Err(transport_unavailable(
                        "connection candidate lost duplicate arbitration",
                    ));
                }
            }
        }
    }

    fn start_connection_actor(&self, slot: Arc<PeerSlot>, candidate: Arc<Candidate>) {
        if candidate.actor_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let broker = self.clone();
        tokio::spawn(async move {
            broker.run_connection_actor(slot, candidate).await;
        });
    }

    async fn run_connection_actor(&self, slot: Arc<PeerSlot>, candidate: Arc<Candidate>) {
        let mut cancel = candidate.cancel.subscribe();
        let mut auth_changes = self
            .inner
            .authorization
            .admit(candidate.remote)
            .ok()
            .and_then(|admission| {
                candidate
                    .inbound_authorization
                    .is_none_or(|accepted| accepted == admission.snapshot)
                    .then_some((admission.snapshot, admission.changes))
            });
        if *cancel.borrow() || (candidate.side == CandidateSide::Inbound && auth_changes.is_none())
        {
            candidate.cancel(ConnectionCloseReason::Unauthorized);
            self.candidate_closed(&slot, candidate.key).await;
            return;
        }
        let mut path_events = candidate.connection.path_events();
        let mut handlers = JoinSet::new();
        self.observe_current_path(&candidate);

        loop {
            tokio::select! {
                biased;
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() {
                        break;
                    }
                }
                stale = wait_for_authorization_change(&mut auth_changes) => {
                    if stale {
                        candidate.cancel(ConnectionCloseReason::Unauthorized);
                        break;
                    }
                }
                _ = candidate.connection.closed() => break,
                joined = handlers.join_next(), if !handlers.is_empty() => {
                    let _ = joined;
                }
                event = path_events.next() => {
                    match event {
                        Some(event) => self.observe_path_event(&candidate, event),
                        None => break,
                    }
                }
                accepted = candidate.connection.accept_bi() => {
                    let Ok((mut send, mut recv)) = accepted else {
                        break;
                    };
                    match try_admit_service_handler(
                        &self.inner.lifecycle,
                        &candidate.admission.handlers,
                        &self.inner.admission.global_stream_handlers,
                    ) {
                        Ok(handler_permits) => {
                            let metric = StreamMetricGuard::new(
                                Arc::clone(&self.inner.metrics),
                                Arc::clone(&slot.active_streams),
                            );
                            let Ok(metric) = metric else {
                                reject_stream(&mut send, &mut recv, b"stream overloaded");
                                continue;
                            };
                            let broker = self.clone();
                            let remote = candidate.remote;
                            let connection_generation = candidate
                                .inbound_authorization
                                .map(|snapshot| snapshot.generation);
                            handlers.spawn(async move {
                                let _handler_permits = handler_permits;
                                let _metric = metric;
                                broker
                                    .handle_service_stream(
                                        remote,
                                        connection_generation,
                                        send,
                                        recv,
                                    )
                                    .await;
                            });
                        }
                        Err(_) => reject_stream(&mut send, &mut recv, b"stream overloaded"),
                    }
                }
            }
        }
        handlers.abort_all();
        while handlers.join_next().await.is_some() {}
        self.candidate_closed(&slot, candidate.key).await;
    }

    async fn handle_service_stream(
        &self,
        remote: DeviceId,
        connection_generation: Option<AuthGeneration>,
        mut send: SendStream,
        mut recv: RecvStream,
    ) {
        if self.inner.lifecycle.is_quiescing() {
            reject_stream(&mut send, &mut recv, b"transport stopping");
            return;
        }
        // Receiver-side directionality: an outbound-only known device is not
        // implicitly authorized merely because QUIC permits reverse streams.
        let accepted_generation = match receiver_generation_for_stream(
            &self.inner.authorization,
            remote,
            connection_generation,
        ) {
            Ok(generation) => generation,
            Err(_) => {
                reject_stream(&mut send, &mut recv, b"not authorized");
                return;
            }
        };
        let deadline = Instant::now() + self.inner.limits.first_frame_deadline;
        #[cfg(unix)]
        if let Some(handler) = self.inner.service_handler.get() {
            if self.inner.lifecycle.is_quiescing() {
                reject_stream(&mut send, &mut recv, b"transport stopping");
                return;
            }
            let stream = InboundAuthenticatedStream {
                send,
                recv,
                remote_device_id: remote,
                accepted_generation,
            };
            let _ = handler.handle_service_stream(stream, deadline).await;
            return;
        }
        #[cfg(not(unix))]
        let _ = accepted_generation;

        // Isolated M5-M6 composition intentionally retains the typed fallback.
        // Only this no-handler branch owns its compatibility pre-read.
        let frame = match read_service_frame(&mut recv, deadline).await {
            Ok(frame) => frame,
            Err(_) => {
                reject_stream(&mut send, &mut recv, b"invalid service frame");
                return;
            }
        };
        if !is_remote_service_kind(frame.kind) {
            reject_stream(&mut send, &mut recv, b"invalid service kind");
            return;
        }
        let response = v2::ServiceError {
            code: DomainErrorKind::ServiceNotImplemented.code().to_owned(),
            message: "remote Session service is not implemented in M5-M6".to_owned(),
        };
        let bytes = match encode_message(
            WireKind::ServiceErrorResponse,
            frame.request_id,
            0,
            &response,
        ) {
            Ok(bytes) => bytes,
            Err(_) => {
                reject_stream(&mut send, &mut recv, b"service response unavailable");
                return;
            }
        };
        if timeout_until(deadline, send.write_all(&bytes))
            .await
            .is_ok_and(|result| result.is_ok())
        {
            let _ = send.finish();
        } else {
            reject_stream(&mut send, &mut recv, b"service response failed");
        }
    }

    async fn candidate_closed(&self, slot: &Arc<PeerSlot>, key: ConnectionCandidateKey) {
        let removed = {
            let mut state = slot.state.lock().await;
            let removed = state.candidates.remove(key);
            if state.candidates.is_empty() {
                state.remote_acceptance = None;
            }
            removed
        };
        if let Some(candidate) = removed {
            candidate.set_primary(false, "transport_closed");
            candidate.cancel(ConnectionCloseReason::EndpointReset);
        }
        slot.changed.notify_waiters();
        if slot.demand.count() > 0 && !self.inner.lifecycle.is_quiescing() {
            let _ = self.ensure_dial_worker(Arc::clone(slot)).await;
        }
    }

    async fn remove_candidate(
        &self,
        slot: &Arc<PeerSlot>,
        key: ConnectionCandidateKey,
        reason: ConnectionCloseReason,
    ) {
        let removed = {
            let mut state = slot.state.lock().await;
            let removed = state.candidates.remove(key);
            if state.candidates.is_empty() {
                state.remote_acceptance = None;
            }
            removed
        };
        if let Some(candidate) = removed {
            candidate.set_primary(false, reason.code());
            candidate.cancel(reason);
        }
        slot.changed.notify_waiters();
    }

    fn persist_verified_handshake(&self, candidate: &Candidate) {
        let store = self.inner.store.clone();
        let authorization = self.inner.authorization.clone();
        let remote = candidate.remote;
        let route = candidate.verified_relay.clone();
        tokio::spawn(async move {
            let now = now_unix_i64();
            if authorization.admit(remote).is_ok() {
                let deadline = Instant::now() + Duration::from_secs(5);
                let _ = store
                    .run_blocking_until(deadline, move |store, deadline| {
                        store.set_last_seen(remote, now, deadline)
                    })
                    .await;
            }
            if let Some(relay) = route {
                persist_verified_route(store, remote, relay, now).await;
            }
        });
    }

    fn observe_current_path(&self, candidate: &Candidate) {
        if !candidate.primary.load(Ordering::Acquire) {
            return;
        }
        let (kind, relay) = candidate
            .connection
            .paths()
            .iter()
            .find(|path| path.is_selected())
            .map_or((PathKind::Unknown, None), |path| {
                classify_transport_addr(path.remote_addr())
            });
        self.inner.metrics.set_path(candidate.remote, kind);
        if let Some(relay) = relay {
            let store = self.inner.store.clone();
            let remote = candidate.remote;
            tokio::spawn(async move {
                persist_verified_route(store, remote, relay, now_unix_i64()).await;
            });
        }
    }

    fn observe_path_event(&self, candidate: &Candidate, event: PathEvent) {
        if !candidate.primary.load(Ordering::Acquire) {
            return;
        }
        match event {
            PathEvent::Selected { remote_addr, .. } => {
                let (kind, relay) = classify_transport_addr(&remote_addr);
                self.inner.metrics.set_path(candidate.remote, kind);
                if let Some(relay) = relay {
                    let store = self.inner.store.clone();
                    let remote = candidate.remote;
                    tokio::spawn(async move {
                        persist_verified_route(store, remote, relay, now_unix_i64()).await;
                    });
                }
            }
            PathEvent::Closed { .. } | PathEvent::Opened { .. } | PathEvent::Lagged { .. } => {
                self.observe_current_path(candidate);
            }
            _ => self.observe_current_path(candidate),
        }
    }
}

impl Candidate {
    #[allow(clippy::too_many_arguments)]
    fn new(
        key: ConnectionCandidateKey,
        remote: DeviceId,
        connection: Connection,
        side: CandidateSide,
        inbound_authorization: Option<AuthorizationSnapshot>,
        verified_relay: Option<RelayHint>,
        connection_permit: OwnedSemaphorePermit,
        metrics: Arc<BrokerMetrics>,
        limits: TransportLimits,
    ) -> Result<Arc<Self>, DaemonError> {
        let metric = ConnectionMetricGuard::new(Arc::clone(&metrics))?;
        let (cancel, _) = watch::channel(false);
        Ok(Arc::new(Self {
            key,
            remote,
            connection,
            side,
            inbound_authorization,
            verified_relay,
            cancel,
            actor_started: AtomicBool::new(false),
            primary: AtomicBool::new(false),
            admission: CandidateAdmission::new(limits),
            metrics,
            _connection_permit: connection_permit,
            _metric: metric,
        }))
    }

    fn set_primary(&self, primary: bool, reason: &'static str) {
        let previous = self.primary.swap(primary, Ordering::AcqRel);
        if previous == primary {
            return;
        }
        if primary {
            if increment_atomic(&self.metrics.primary).is_err() {
                self.primary.store(false, Ordering::Release);
                return;
            }
            self.metrics.set_path(self.remote, PathKind::Unknown);
        } else {
            decrement_atomic(&self.metrics.primary);
            self.metrics.remove_path(self.remote);
        }
        self.metrics.publish();
        tracing::info!(
            component = "connection",
            operation = if primary {
                "primary_established"
            } else {
                "primary_closed"
            },
            reason,
            "Primary connection changed"
        );
    }

    fn cancel(&self, reason: ConnectionCloseReason) {
        self.cancel.send_replace(true);
        let (code, message) = reason.wire();
        self.connection.close(code, message);
    }
}

impl Drop for Candidate {
    fn drop(&mut self) {
        if self.primary.swap(false, Ordering::AcqRel) {
            decrement_atomic(&self.metrics.primary);
            self.metrics.remove_path(self.remote);
            self.metrics.publish();
        }
    }
}

impl ConnectionMetricGuard {
    fn new(metrics: Arc<BrokerMetrics>) -> Result<Self, DaemonError> {
        increment_atomic(&metrics.authenticated).map_err(|()| {
            resource_exhausted("network observation counter exhausted its address space")
        })?;
        metrics.publish();
        Ok(Self { metrics })
    }
}

impl Drop for ConnectionMetricGuard {
    fn drop(&mut self) {
        decrement_atomic(&self.metrics.authenticated);
        self.metrics.publish();
    }
}

impl StreamMetricGuard {
    fn new(
        metrics: Arc<BrokerMetrics>,
        peer_streams: Arc<AtomicUsize>,
    ) -> Result<Self, DaemonError> {
        increment_atomic(&peer_streams).map_err(|()| {
            resource_exhausted("peer stream observation counter exhausted its address space")
        })?;
        if increment_atomic(&metrics.streams).is_err() {
            decrement_atomic(&peer_streams);
            return Err(resource_exhausted(
                "network observation counter exhausted its address space",
            ));
        }
        metrics.publish();
        Ok(Self {
            metrics,
            peer_streams,
        })
    }
}

impl Drop for StreamMetricGuard {
    fn drop(&mut self) {
        decrement_atomic(&self.peer_streams);
        decrement_atomic(&self.metrics.streams);
        self.metrics.publish();
    }
}

impl BrokerMetrics {
    fn publish(&self) {
        let (direct, relay) = {
            let paths = mutex_lock(&self.paths);
            (
                paths
                    .values()
                    .filter(|kind| **kind == PathKind::Direct)
                    .count(),
                paths
                    .values()
                    .filter(|kind| **kind == PathKind::Relay)
                    .count(),
            )
        };
        self.reporter.transport_metrics(
            self.authenticated.load(Ordering::Acquire),
            self.primary.load(Ordering::Acquire),
            self.streams.load(Ordering::Acquire),
            direct,
            relay,
        );
    }

    fn set_path(&self, remote: DeviceId, kind: PathKind) {
        mutex_lock(&self.paths).insert(remote, kind);
        self.publish();
    }

    fn remove_path(&self, remote: DeviceId) {
        mutex_lock(&self.paths).remove(&remote);
        self.publish();
    }

    fn path(&self, remote: DeviceId) -> PathKind {
        mutex_lock(&self.paths)
            .get(&remote)
            .copied()
            .unwrap_or_default()
    }
}

async fn write_handshake_message<M: prost::Message>(
    send: &mut SendStream,
    kind: WireKind,
    message: &M,
    deadline: Instant,
) -> Result<(), DaemonError> {
    let bytes = encode_message(kind, 0, 0, message).map_err(protocol_error)?;
    timeout_until(deadline, send.write_all(&bytes))
        .await?
        .map_err(|_| transport_unavailable("handshake stream write failed"))
}

async fn read_handshake_frame(
    recv: &mut RecvStream,
    maximum_body_bytes: usize,
    deadline: Instant,
) -> Result<DecodedFrame, DaemonError> {
    read_one_frame(recv, maximum_body_bytes, deadline).await
}

async fn read_service_frame(
    recv: &mut RecvStream,
    deadline: Instant,
) -> Result<DecodedFrame, DaemonError> {
    read_one_frame(recv, zterm_proto::MAX_FRAME_BYTES, deadline).await
}

async fn read_one_frame<Reader>(
    recv: &mut Reader,
    maximum_body_bytes: usize,
    deadline: Instant,
) -> Result<DecodedFrame, DaemonError>
where
    Reader: AsyncRead + Unpin,
{
    timeout_until(deadline, async {
        let mut decoder = FrameDecoder::with_maximum_body_bytes(maximum_body_bytes);
        let mut buffer = [0_u8; 4096];
        loop {
            let read = AsyncReadExt::read(recv, &mut buffer)
                .await
                .map_err(|_| transport_unavailable("stream read failed"))?;
            if read == 0 {
                decoder.finish().map_err(protocol_error)?;
                return Err(DaemonError::new(
                    DomainErrorKind::MalformedFrame,
                    "stream ended before its first frame",
                ));
            }
            let frames = decoder.feed(&buffer[..read]).map_err(protocol_error)?;
            match frames.len() {
                0 => {}
                1 => {
                    return frames.into_iter().next().ok_or_else(|| {
                        DaemonError::new(
                            DomainErrorKind::MalformedFrame,
                            "decoder returned an inconsistent frame count",
                        )
                    });
                }
                _ => {
                    return Err(DaemonError::new(
                        DomainErrorKind::MalformedFrame,
                        "stream sent multiple frames before dispatch",
                    ));
                }
            }
        }
    })
    .await?
}

fn reject_stream(send: &mut SendStream, recv: &mut RecvStream, _reason: &'static [u8]) {
    let code = VarInt::from_u32(STREAM_REJECTED);
    let _ = recv.stop(code);
    let _ = send.reset(code);
}

fn close_connection(connection: &Connection, code: u32, reason: &'static [u8]) {
    connection.close(VarInt::from_u32(code), reason);
}

fn admit_inbound_before_payload(
    authorization: &AuthorizationRegistry,
    remote: DeviceId,
) -> Result<crate::authorization::Admission, DaemonError> {
    authorization.admit(remote)
}

fn recheck_inbound_admission(
    authorization: &AuthorizationRegistry,
    remote: DeviceId,
    accepted: zterm_core::AuthorizationSnapshot,
) -> Result<crate::authorization::Admission, DaemonError> {
    let current = authorization.admit(remote)?;
    if current.snapshot != accepted {
        return Err(DaemonError::new(
            DomainErrorKind::Unauthorized,
            "device is not authorized to control this host",
        ));
    }
    Ok(current)
}

fn receiver_generation_for_stream(
    authorization: &AuthorizationRegistry,
    remote: DeviceId,
    connection_generation: Option<AuthGeneration>,
) -> Result<AuthGeneration, DaemonError> {
    let current = authorization.admit(remote)?;
    if connection_generation.is_some_and(|accepted| accepted != current.snapshot.generation) {
        return Err(DaemonError::new(
            DomainErrorKind::Unauthorized,
            "device is not authorized to control this host",
        ));
    }
    Ok(current.snapshot.generation)
}

async fn wait_for_authorization_change(
    admission: &mut Option<(
        zterm_core::AuthorizationSnapshot,
        watch::Receiver<zterm_core::AuthorizationSnapshot>,
    )>,
) -> bool {
    let Some((accepted, changes)) = admission else {
        return pending::<bool>().await;
    };
    match changes.changed().await {
        Ok(()) => {
            let current = *changes.borrow_and_update();
            current != *accepted || current.status != AuthorizationStatus::Authorized
        }
        Err(_) => true,
    }
}

fn is_remote_service_kind(kind: WireKind) -> bool {
    matches!(
        kind,
        WireKind::SessionListRequest
            | WireKind::SessionCreateRequest
            | WireKind::SessionRenameRequest
            | WireKind::SessionCloseRequest
            | WireKind::SessionTakeoverRequest
            | WireKind::SessionOperationLeaseRequest
            | WireKind::TerminalAttachRequest
            | WireKind::TerminalInput
            | WireKind::TerminalResize
            | WireKind::TerminalDetach
            | WireKind::TerminalSnapshotApplied
            | WireKind::TerminalSyncRequest
            | WireKind::TerminalHistoryWindowRequest
    )
}

fn selected_relay(connection: &Connection) -> Option<RelayHint> {
    connection
        .paths()
        .iter()
        .find(|path| path.is_selected())
        .and_then(|path| classify_transport_addr(path.remote_addr()).1)
}

fn validate_pair_tls_connection(
    connection: &Connection,
    expected_remote: DeviceId,
) -> Result<(), DaemonError> {
    validate_pair_tls_metadata(
        expected_remote,
        device_from_endpoint_id(connection.remote_id()),
        connection.alpn(),
    )
}

fn validate_pair_tls_metadata(
    expected_remote: DeviceId,
    authenticated_remote: DeviceId,
    alpn: &[u8],
) -> Result<(), DaemonError> {
    if alpn != ZTERM_PAIR_ALPN {
        return Err(DaemonError::new(
            DomainErrorKind::WireMajorMismatch,
            "pairing connection negotiated an unexpected ALPN",
        ));
    }
    if authenticated_remote != expected_remote {
        return Err(DaemonError::new(
            DomainErrorKind::Unauthorized,
            "connected Iroh identity did not match the pairing ticket host",
        ));
    }
    Ok(())
}

fn should_try_next_pair_route(error: &DaemonError, overall_deadline: Instant) -> bool {
    Instant::now() < overall_deadline
        && matches!(
            error.kind(),
            DomainErrorKind::TransportUnavailable | DomainErrorKind::DeadlineExceeded
        )
}

/// Exercises the broker's real candidate registry under one of the two
/// opposite duplicate-arrival schedules without opening a socket.
#[doc(hidden)]
pub fn duplicate_connection_test_evidence(
    lower: ConnectionCandidateKey,
    higher: ConnectionCandidateKey,
    higher_arrives_first: bool,
) -> Result<DuplicateConnectionTestEvidence, DaemonError> {
    if lower >= higher {
        return Err(DaemonError::new(
            DomainErrorKind::MalformedFrame,
            "duplicate test keys must be supplied in strict winner order",
        ));
    }

    let mut provisional = CandidateRegistry::default();
    provisional
        .register(lower, lower)
        .map_err(|_| duplicate_test_error())?;
    provisional
        .register(higher, higher)
        .map_err(|_| duplicate_test_error())?;
    let redial_suppressed_while_provisional =
        !peer_needs_outbound_attempt(&provisional, None, |_| false);

    let mut candidates = CandidateRegistry::default();
    let loser_count = if higher_arrives_first {
        candidates
            .register(higher, higher)
            .map_err(|_| duplicate_test_error())?;
        match candidates.mark_ready_and_decide(higher) {
            CandidateDecision::Promoted(losers) if losers.is_empty() => {}
            _ => return Err(duplicate_test_error()),
        }
        candidates
            .register(lower, lower)
            .map_err(|_| duplicate_test_error())?;
        match candidates.mark_ready_and_decide(lower) {
            CandidateDecision::Promoted(losers) => losers.len(),
            _ => return Err(duplicate_test_error()),
        }
    } else {
        candidates
            .register(lower, lower)
            .map_err(|_| duplicate_test_error())?;
        candidates
            .register(higher, higher)
            .map_err(|_| duplicate_test_error())?;
        if !matches!(
            candidates.mark_ready_and_decide(higher),
            CandidateDecision::Wait
        ) {
            return Err(duplicate_test_error());
        }
        match candidates.mark_ready_and_decide(lower) {
            CandidateDecision::Promoted(losers) => losers.len(),
            _ => return Err(duplicate_test_error()),
        }
    };

    let primary = candidates.primary();
    let remaining_candidate_count = candidates.len();
    let accepted_generation = AuthGeneration::new(1).ok_or_else(duplicate_test_error)?;
    let redial_suppressed_after_confirmation =
        !peer_needs_outbound_attempt(&candidates, Some(accepted_generation), |_| false);
    let _closed = candidates.take_all();
    let empty_after_peer_close = candidates.is_empty() && candidates.primary().is_none();

    Ok(DuplicateConnectionTestEvidence {
        primary,
        remaining_candidate_count,
        loser_count,
        redial_suppressed_while_provisional,
        redial_suppressed_after_confirmation,
        empty_after_peer_close,
    })
}

/// Exercises the broker's real global, peer, connection, and metric permits
/// with capacity one and no transport resources.
#[doc(hidden)]
#[must_use]
pub fn stream_limit_test_evidence() -> StreamLimitTestEvidence {
    let limits = TransportLimits {
        max_stream_handlers_global: 1,
        max_open_stream_queue_per_connection: 1,
        max_bi_streams_per_connection: 1,
        max_stream_handlers_per_connection: 1,
        ..TransportLimits::default()
    };
    let broker = BrokerAdmission::new(limits);
    let global = Arc::clone(&broker.global_stream_handlers)
        .try_acquire_owned()
        .ok();
    let global_overflow_rejected = global.is_some()
        && Arc::clone(&broker.global_stream_handlers)
            .try_acquire_owned()
            .is_err();
    drop(global);
    let global_capacity_released = broker.global_stream_handlers.available_permits() == 1;

    let first_peer = PeerAdmission::new(limits);
    let second_peer = PeerAdmission::new(limits);
    let first_peer_permit = Arc::clone(&first_peer.open_queue).try_acquire_owned().ok();
    let peer_overflow_rejected = first_peer_permit.is_some()
        && Arc::clone(&first_peer.open_queue)
            .try_acquire_owned()
            .is_err();
    let second_peer_permit = Arc::clone(&second_peer.open_queue).try_acquire_owned().ok();
    let peer_isolated = first_peer_permit.is_some() && second_peer_permit.is_some();
    drop((first_peer_permit, second_peer_permit));
    let peer_capacity_released = first_peer.open_queue.available_permits() == 1
        && second_peer.open_queue.available_permits() == 1;

    let first_connection = CandidateAdmission::new(limits);
    let second_connection = CandidateAdmission::new(limits);
    let first_stream = Arc::clone(&first_connection.streams)
        .try_acquire_owned()
        .ok();
    let first_handler = Arc::clone(&first_connection.handlers)
        .try_acquire_owned()
        .ok();
    let connection_overflow_rejected = first_stream.is_some()
        && first_handler.is_some()
        && Arc::clone(&first_connection.streams)
            .try_acquire_owned()
            .is_err()
        && Arc::clone(&first_connection.handlers)
            .try_acquire_owned()
            .is_err();
    let second_stream = Arc::clone(&second_connection.streams)
        .try_acquire_owned()
        .ok();
    let second_handler = Arc::clone(&second_connection.handlers)
        .try_acquire_owned()
        .ok();
    let connection_isolated = first_stream.is_some()
        && first_handler.is_some()
        && second_stream.is_some()
        && second_handler.is_some();
    drop((first_stream, first_handler, second_stream, second_handler));
    let connection_capacity_released = first_connection.streams.available_permits() == 1
        && first_connection.handlers.available_permits() == 1
        && second_connection.streams.available_permits() == 1
        && second_connection.handlers.available_permits() == 1;

    let (reporter, observer) = NetworkReporter::initializing(DeviceId::from_array([0xa0; 32]));
    let metrics = Arc::new(BrokerMetrics {
        authenticated: AtomicUsize::new(0),
        primary: AtomicUsize::new(0),
        streams: AtomicUsize::new(0),
        paths: Mutex::new(BTreeMap::new()),
        reporter,
    });
    let first_peer_streams = Arc::new(AtomicUsize::new(0));
    let second_peer_streams = Arc::new(AtomicUsize::new(0));
    let first_metric =
        StreamMetricGuard::new(Arc::clone(&metrics), Arc::clone(&first_peer_streams)).ok();
    let second_metric =
        StreamMetricGuard::new(Arc::clone(&metrics), Arc::clone(&second_peer_streams)).ok();
    let metric_both_acquired = first_metric.is_some()
        && second_metric.is_some()
        && observer.snapshot().active_stream_count == 2;
    drop(first_metric);
    let metric_peer_isolated = metric_both_acquired
        && first_peer_streams.load(Ordering::Acquire) == 0
        && second_peer_streams.load(Ordering::Acquire) == 1
        && observer.snapshot().active_stream_count == 1;
    drop(second_metric);
    let metric_capacity_released = first_peer_streams.load(Ordering::Acquire) == 0
        && second_peer_streams.load(Ordering::Acquire) == 0
        && observer.snapshot().active_stream_count == 0;

    StreamLimitTestEvidence {
        global_overflow_rejected,
        global_capacity_released,
        peer_overflow_rejected,
        peer_isolated,
        peer_capacity_released,
        connection_overflow_rejected,
        connection_isolated,
        connection_capacity_released,
        metric_peer_isolated,
        metric_capacity_released,
    }
}

/// Projects path transitions through the broker's real redacted metric owner.
/// Direct socket addresses are accepted only as input and never retained or
/// returned by the evidence value.
#[doc(hidden)]
#[must_use]
pub fn path_observation_test_evidence(
    local: DeviceId,
    remote: DeviceId,
    addresses: &[TransportAddr],
) -> PathObservationTestEvidence {
    let (reporter, observer) = NetworkReporter::initializing(local);
    let metrics = BrokerMetrics {
        authenticated: AtomicUsize::new(0),
        primary: AtomicUsize::new(0),
        streams: AtomicUsize::new(0),
        paths: Mutex::new(BTreeMap::new()),
        reporter,
    };
    let mut timeline = Vec::with_capacity(addresses.len().saturating_add(1));
    let mut persistable_relays = Vec::with_capacity(addresses.len());
    for address in addresses {
        let (kind, relay) = classify_transport_addr(address);
        metrics.set_path(remote, kind);
        timeline.push(metrics.path(remote));
        persistable_relays.push(relay);
    }
    let selected_observation = observer.snapshot();
    metrics.remove_path(remote);
    timeline.push(metrics.path(remote));
    let cleared_observation = observer.snapshot();
    PathObservationTestEvidence {
        timeline,
        persistable_relays,
        selected_observation,
        cleared_observation,
    }
}

fn duplicate_test_error() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::TransportUnavailable,
        "duplicate candidate test schedule violated broker invariants",
    )
}

fn classify_transport_addr(address: &TransportAddr) -> (PathKind, Option<RelayHint>) {
    match address {
        TransportAddr::Relay(url) => (PathKind::Relay, RelayHint::new(url.to_string()).ok()),
        TransportAddr::Ip(_) => (PathKind::Direct, None),
        _ => (PathKind::Unknown, None),
    }
}

async fn persist_verified_route(
    store: StoreHandle,
    remote: DeviceId,
    relay: RelayHint,
    now_unix: i64,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let _ = store
        .run_blocking_until(deadline, move |store, deadline| {
            store.set_known_route(
                remote,
                RelayRouteCache {
                    relay_hints: vec![relay],
                    verified_at_unix: now_unix,
                },
                deadline,
            )
        })
        .await;
}

fn now_unix_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(i64::MAX)
}

fn random_attempt_id() -> Result<ConnectionAttemptId, DaemonError> {
    let mut bytes = [0_u8; 16];
    SystemRandom::new().fill(&mut bytes).map_err(|_| {
        DaemonError::new(
            DomainErrorKind::TransportUnavailable,
            "operating-system randomness is unavailable for a connection attempt",
        )
    })?;
    Ok(ConnectionAttemptId::from_array(bytes))
}

fn retry_delay(attempt: u32, remote: DeviceId) -> Duration {
    let exponent = attempt.min(6);
    let multiplier = 1_u32 << exponent;
    let base = RETRY_BASE.saturating_mul(multiplier).min(RETRY_CAP);
    let bytes = remote.as_bytes();
    let mixed =
        u32::from(bytes[(attempt as usize) % bytes.len()]) ^ attempt.wrapping_mul(0x9e37_79b9);
    let jitter_ceiling = base / 5;
    let jitter = if jitter_ceiling.is_zero() {
        Duration::ZERO
    } else {
        Duration::from_nanos(
            u64::from(mixed) % u64::try_from(jitter_ceiling.as_nanos()).unwrap_or(u64::MAX),
        )
    };
    base.saturating_add(jitter).min(RETRY_CAP)
}

fn validate_limits(limits: TransportLimits) -> Result<(), DaemonError> {
    limits
        .validate()
        .map_err(|error| DaemonError::new(DomainErrorKind::ResourceExhausted, error.to_string()))?;
    limits
        .max_unauthenticated_connections
        .checked_add(limits.max_pairing_handshakes)
        .ok_or_else(|| {
            DaemonError::new(
                DomainErrorKind::ResourceExhausted,
                "pre-authentication connection limit overflow",
            )
        })?;
    for (field, value) in [
        (
            "max_bi_streams_per_connection",
            limits.max_bi_streams_per_connection,
        ),
        (
            "max_stream_handlers_per_connection",
            limits.max_stream_handlers_per_connection,
        ),
    ] {
        if u32::try_from(value).is_err() {
            return Err(DaemonError::new(
                DomainErrorKind::ResourceExhausted,
                format!("transport limit {field} exceeds QUIC's u32 boundary"),
            ));
        }
    }
    Ok(())
}

fn configure_normal_connection_limits(connection: &Connection, limits: TransportLimits) {
    // `ConnectionBroker::new` rejects values outside QUIC's u32 boundary.
    // Keeping the conversion defensive avoids a panic if this helper is ever
    // reused at another composition boundary.
    let maximum_bi_streams =
        u32::try_from(limits.max_bi_streams_per_connection).unwrap_or(u32::MAX);
    connection.set_max_concurrent_bi_streams(VarInt::from_u32(maximum_bi_streams));
    // The product protocol owns only bidirectional streams. Advertising zero
    // prevents an authenticated peer from accumulating unconsumed uni streams.
    connection.set_max_concurrent_uni_streams(VarInt::from_u32(0));
}

fn protocol_error(error: zterm_proto::ProtocolError) -> DaemonError {
    use zterm_proto::ProtocolError;
    let kind = match error {
        ProtocolError::WireMajorMismatch { .. } => DomainErrorKind::WireMajorMismatch,
        ProtocolError::UnknownKind(_) => DomainErrorKind::UnknownKind,
        ProtocolError::FrameTooLarge(_) => DomainErrorKind::FrameTooLarge,
        ProtocolError::ControlPayloadTooLarge(_) => DomainErrorKind::ControlPayloadTooLarge,
        ProtocolError::MalformedVarint
        | ProtocolError::TruncatedFrame
        | ProtocolError::MalformedProtobuf(_)
        | ProtocolError::UnexpectedKind { .. }
        | ProtocolError::InvalidIdentifier(_)
        | ProtocolError::InvalidTerminalSize { .. }
        | ProtocolError::InvalidTerminalSurface(_)
        | ProtocolError::InvalidTerminalSemanticField(_) => DomainErrorKind::MalformedFrame,
    };
    DaemonError::new(kind, error.to_string())
}

fn increment_atomic(counter: &AtomicUsize) -> Result<(), ()> {
    increment_atomic_previous(counter).map(|_| ())
}

fn increment_atomic_previous(counter: &AtomicUsize) -> Result<usize, ()> {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .map_err(|_| ())
}

fn decrement_atomic(counter: &AtomicUsize) {
    let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        current.checked_sub(1)
    });
}

async fn acquire_until(
    semaphore: Arc<Semaphore>,
    deadline: Instant,
    detail: &'static str,
) -> Result<OwnedSemaphorePermit, DaemonError> {
    timeout_until(deadline, semaphore.acquire_owned())
        .await?
        .map_err(|_| transport_unavailable(detail))
}

async fn wait_for_notify(notify: &Notify, deadline: Instant) -> Result<(), DaemonError> {
    timeout_until(deadline, notify.notified()).await
}

async fn wait_for_notify_optional(notify: &Notify, duration: Duration) -> Result<(), DaemonError> {
    timeout_until(Instant::now() + duration, notify.notified()).await
}

async fn timeout_until<F>(deadline: Instant, future: F) -> Result<F::Output, DaemonError>
where
    F: std::future::Future,
{
    if Instant::now() >= deadline {
        return Err(deadline_exceeded("transport operation deadline elapsed"));
    }
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future)
        .await
        .map_err(|_| deadline_exceeded("transport operation deadline elapsed"))
}

async fn timeout_or_pair_shutdown<F>(
    deadline: Instant,
    mut shutdown: watch::Receiver<bool>,
    future: F,
) -> Result<F::Output, DaemonError>
where
    F: std::future::Future,
{
    if *shutdown.borrow() {
        return Err(cancelled("pairing transport is stopping"));
    }
    if Instant::now() >= deadline {
        return Err(deadline_exceeded("pairing operation deadline elapsed"));
    }
    tokio::select! {
        biased;
        changed = shutdown.changed() => {
            let _ = changed;
            Err(cancelled("pairing transport is stopping"))
        }
        result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future) => {
            result.map_err(|_| deadline_exceeded("pairing operation deadline elapsed"))
        }
    }
}

fn is_retryable(kind: DomainErrorKind) -> bool {
    matches!(
        kind,
        DomainErrorKind::AddressUnavailable
            | DomainErrorKind::TransportUnavailable
            | DomainErrorKind::DeadlineExceeded
            | DomainErrorKind::ResourceExhausted
            | DomainErrorKind::StoreUnavailable
    )
}

fn peer_needs_outbound_attempt<T>(
    candidates: &CandidateRegistry<T>,
    remote_acceptance: Option<AuthGeneration>,
    is_outbound: impl Fn(&T) -> bool,
) -> bool {
    match candidates.primary() {
        None => candidates.is_empty(),
        Some(_) => remote_acceptance.is_none() && !candidates.values().any(is_outbound),
    }
}

fn mutex_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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

fn bounded_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(unix)]
fn round_rtt_millis(rtt: Duration) -> u32 {
    let rounded = rtt.as_nanos().saturating_add(500_000) / 1_000_000;
    u32::try_from(rounded).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::future::{Future, poll_fn};
    use std::sync::Arc;

    use iroh::{RelayUrl, TransportAddr};
    use tokio::sync::oneshot;
    use zterm_core::{
        AuthorizationSnapshot, ConnectionAttemptId, DeviceDisplayName, TransportLimits,
    };

    use super::*;
    use crate::authorization::AuthorizationRegistry;
    use crate::network::NetworkReporter;
    use crate::store::DeviceAuthorization;

    #[cfg(unix)]
    struct NoopRemoteServiceHandler;

    #[cfg(unix)]
    impl RemoteServiceHandler for NoopRemoteServiceHandler {
        fn handle_service_stream(
            &self,
            _stream: InboundAuthenticatedStream,
            _first_frame_deadline: Instant,
        ) -> RemoteServiceHandlerFuture {
            Box::pin(async { Ok(()) })
        }
    }

    fn device(byte: u8) -> DeviceId {
        DeviceId::from_array([byte; 32])
    }

    fn key(initiator: u8, attempt: u8) -> ConnectionCandidateKey {
        ConnectionCandidateKey::new(
            device(initiator),
            ConnectionAttemptId::from_array([attempt; 16]),
        )
    }

    fn relay(url: &str) -> RelayHint {
        RelayHint::new(url).expect("test relay is valid")
    }

    #[cfg(unix)]
    #[test]
    fn selected_path_rtt_rounds_to_nearest_millisecond_and_clamps() {
        assert_eq!(round_rtt_millis(Duration::from_micros(499)), 0);
        assert_eq!(round_rtt_millis(Duration::from_micros(500)), 1);
        assert_eq!(round_rtt_millis(Duration::from_micros(1_499)), 1);
        assert_eq!(round_rtt_millis(Duration::from_micros(1_500)), 2);
        assert_eq!(round_rtt_millis(Duration::MAX), u32::MAX);
    }

    fn authorization(
        device_id: DeviceId,
        status: AuthorizationStatus,
        generation: u64,
    ) -> DeviceAuthorization {
        DeviceAuthorization {
            device_id,
            display_name: DeviceDisplayName::new("peer").expect("display name"),
            status,
            generation: AuthGeneration::new(generation).expect("non-zero generation"),
            paired_at_unix: 1,
            revoked_at_unix: (status == AuthorizationStatus::Revoked).then_some(2),
            last_seen_at_unix: None,
        }
    }

    #[test]
    fn candidate_registry_converges_from_opposite_registration_orders() {
        let low = key(1, 1);
        let high = key(2, 1);

        let mut high_first = CandidateRegistry::default();
        high_first.register(high, "high").expect("register high");
        assert!(matches!(
            high_first.mark_ready_and_decide(high),
            CandidateDecision::Promoted(ref losers) if losers.is_empty()
        ));
        high_first.register(low, "low").expect("register low");
        assert!(matches!(
            high_first.mark_ready_and_decide(low),
            CandidateDecision::Promoted(ref losers) if losers == &["high"]
        ));
        assert_eq!(high_first.primary(), Some(low));
        assert_eq!(high_first.len(), 1);

        let mut low_first = CandidateRegistry::default();
        low_first.register(low, "low").expect("register low");
        low_first.register(high, "high").expect("register high");
        assert!(matches!(
            low_first.mark_ready_and_decide(high),
            CandidateDecision::Wait
        ));
        assert!(matches!(
            low_first.mark_ready_and_decide(low),
            CandidateDecision::Promoted(ref losers) if losers == &["high"]
        ));
        assert_eq!(low_first.primary(), Some(low));
        assert_eq!(low_first.len(), 1);

        assert_eq!(high_first.primary(), low_first.primary());
        assert!(high_first.register(low, "collision").is_err());
    }

    #[test]
    fn candidate_loser_cleanup_and_peer_close_leave_no_redial_triggering_ghosts() {
        let low = key(1, 1);
        let high = key(2, 1);
        let mut candidates = CandidateRegistry::default();
        candidates.register(low, "low").expect("register low");
        candidates.register(high, "high").expect("register high");
        assert!(matches!(
            candidates.mark_ready_and_decide(high),
            CandidateDecision::Wait
        ));
        assert!(
            !peer_needs_outbound_attempt(&candidates, None, |_| false),
            "a provisional duplicate candidate suppresses redial"
        );

        assert_eq!(candidates.remove(low), Some("low"));
        assert!(matches!(
            candidates.mark_ready_and_decide(high),
            CandidateDecision::Promoted(ref losers) if losers.is_empty()
        ));
        assert_eq!(candidates.primary(), Some(high));
        assert!(
            peer_needs_outbound_attempt(&candidates, None, |_| false),
            "an inbound primary needs one outbound acceptance proof"
        );
        assert!(
            !peer_needs_outbound_attempt(
                &candidates,
                Some(AuthGeneration::new(1).expect("generation")),
                |_| false,
            ),
            "a duplicate loser does not redial after proving acceptance"
        );

        assert_eq!(candidates.take_all(), vec!["high"]);
        assert_eq!(candidates.primary(), None);
        assert!(candidates.is_empty());

        let lifecycle = BrokerLifecycle::default();
        assert!(!lifecycle.is_quiescing());
        lifecycle.begin_quiesce();
        assert!(lifecycle.is_quiescing());
    }

    #[test]
    fn demand_bookkeeping_is_checked_singleflight_and_drops_transient_routes() {
        let demand = DemandState::new();
        let first_route = relay("https://first.example");
        let second_route = relay("https://second.example");

        let (previous, first_lease) = demand
            .acquire(vec![first_route.clone()], 2)
            .expect("first demand");
        assert_eq!(previous, 0);
        let (previous, second_lease) = demand
            .acquire(vec![first_route.clone(), second_route.clone()], 2)
            .expect("second demand");
        assert_eq!(previous, 1);
        assert_eq!(demand.count(), 2);
        assert_eq!(
            demand.routes(),
            vec![first_route.clone(), second_route.clone()]
        );

        let overflow = demand
            .acquire(vec![relay("https://third.example")], 2)
            .expect_err("unique route bound is enforced");
        assert_eq!(overflow.kind(), DomainErrorKind::ResourceExhausted);
        assert_eq!(demand.count(), 2, "failed acquisition rolls back count");

        demand.release(&first_lease);
        assert_eq!(demand.count(), 1);
        assert_eq!(
            demand.routes(),
            vec![first_route.clone(), second_route.clone()]
        );
        demand.release(&second_lease);
        assert_eq!(demand.count(), 0);
        assert!(demand.routes().is_empty());

        let mut peer = PeerState::default();
        assert!(peer.claim_dial_worker());
        assert!(!peer.claim_dial_worker(), "one peer has one dial worker");
        peer.terminal_error = Some(DaemonError::new(DomainErrorKind::Unauthorized, "terminal"));
        peer.begin_demand_cycle(1);
        assert!(peer.terminal_error.is_some());
        peer.begin_demand_cycle(0);
        assert!(
            peer.terminal_error.is_none(),
            "a new user demand retries once"
        );
    }

    #[test]
    fn broker_peer_and_connection_admission_are_bounded_without_sockets() {
        let limits = TransportLimits {
            max_pending_dials: 2,
            max_remote_connections: 2,
            max_stream_handlers_global: 2,
            max_open_stream_queue_per_connection: 2,
            max_bi_streams_per_connection: 2,
            max_stream_handlers_per_connection: 2,
            ..TransportLimits::default()
        };
        let broker = BrokerAdmission::new(limits);
        let peer = PeerAdmission::new(limits);
        let connection = CandidateAdmission::new(limits);

        assert_two_permits_then_full(&broker.pending_dials);
        assert_two_permits_then_full(&broker.authenticated_connections);
        assert_two_permits_then_full(&broker.global_stream_handlers);
        assert_two_permits_then_full(&peer.open_queue);
        assert_two_permits_then_full(&connection.streams);
        assert_two_permits_then_full(&connection.handlers);
    }

    #[test]
    #[cfg(unix)]
    fn remote_service_handler_installs_once_without_transport_ownership() {
        let slot = RemoteServiceHandlerSlot::default();
        assert!(!slot.is_installed());
        slot.install(Arc::new(NoopRemoteServiceHandler))
            .expect("first handler installation");
        assert!(slot.is_installed());
        assert!(slot.get().is_some());
        let error = slot
            .install(Arc::new(NoopRemoteServiceHandler))
            .expect_err("handler ownership cannot be replaced after installation");
        assert_eq!(error.kind(), DomainErrorKind::IdentityStateMismatch);
    }

    #[tokio::test]
    async fn service_handler_admission_quiesce_and_reclamation_use_raii_permits() {
        let lifecycle = BrokerLifecycle::default();
        let connection = Arc::new(Semaphore::new(1));
        let global = Arc::new(Semaphore::new(1));
        let permits = try_admit_service_handler(&lifecycle, &connection, &global)
            .expect("first service handler is admitted");
        assert_eq!(connection.available_permits(), 0);
        assert_eq!(global.available_permits(), 0);
        assert_eq!(
            try_admit_service_handler(&lifecycle, &connection, &global)
                .expect_err("connection-local handler capacity is bounded")
                .kind(),
            DomainErrorKind::ResourceExhausted
        );

        let wait_global = Arc::clone(&global);
        let (first_poll, first_poll_observation) = oneshot::channel();
        let waiting = tokio::spawn(async move {
            let wait = tokio::task::unconstrained(wait_for_service_handlers_until(
                wait_global,
                1,
                Instant::now() + Duration::from_secs(1),
            ));
            tokio::pin!(wait);
            let mut observer = Some(first_poll);
            poll_fn(|context| {
                let result = wait.as_mut().poll(context);
                if let Some(observer) = observer.take() {
                    let _ = observer.send(result.is_pending());
                }
                result
            })
            .await
        });
        assert!(
            first_poll_observation
                .await
                .expect("handler reclamation first-poll result"),
            "reclamation waits while the admitted handler owns its permit"
        );
        drop(permits);
        waiting
            .await
            .expect("handler reclamation task")
            .expect("released handler permit is reclaimed");
        assert_eq!(connection.available_permits(), 1);
        assert_eq!(global.available_permits(), 1);

        lifecycle.begin_quiesce();
        let error = try_admit_service_handler(&lifecycle, &connection, &global)
            .expect_err("new service admission fails closed after quiesce");
        assert_eq!(error.kind(), DomainErrorKind::TransportUnavailable);
        assert_eq!(connection.available_permits(), 1);
        assert_eq!(global.available_permits(), 1);
    }

    #[tokio::test]
    async fn service_handler_panic_releases_only_its_stream_permits() {
        let lifecycle = BrokerLifecycle::default();
        let connection = Arc::new(Semaphore::new(1));
        let global = Arc::new(Semaphore::new(1));
        let permits = try_admit_service_handler(&lifecycle, &connection, &global)
            .expect("panic fixture handler admission");
        let mut handlers = JoinSet::new();
        handlers.spawn(async move {
            let _permits = permits;
            panic!("injected stream-local handler panic");
        });
        let joined = handlers.join_next().await.expect("one handler task joined");
        assert!(joined.is_err(), "the injected handler task panicked");
        assert!(!lifecycle.is_quiescing());
        assert_eq!(connection.available_permits(), 1);
        assert_eq!(global.available_permits(), 1);
        drop(
            try_admit_service_handler(&lifecycle, &connection, &global)
                .expect("another independent handler remains admissible"),
        );
    }

    fn assert_two_permits_then_full(semaphore: &Arc<Semaphore>) {
        let first = Arc::clone(semaphore)
            .try_acquire_owned()
            .expect("first permit");
        let second = Arc::clone(semaphore)
            .try_acquire_owned()
            .expect("second permit");
        assert!(
            Arc::clone(semaphore).try_acquire_owned().is_err(),
            "third permit must be rejected"
        );
        drop((first, second));
        assert_eq!(semaphore.available_permits(), 2);
    }

    #[test]
    fn per_peer_stream_observation_tracks_raii_lifetimes() {
        let local = device(0x31);
        let (reporter, observer) = NetworkReporter::initializing(local);
        let metrics = Arc::new(BrokerMetrics {
            authenticated: AtomicUsize::new(0),
            primary: AtomicUsize::new(0),
            streams: AtomicUsize::new(0),
            paths: Mutex::new(BTreeMap::new()),
            reporter,
        });
        let peer_streams = Arc::new(AtomicUsize::new(0));
        let first = StreamMetricGuard::new(Arc::clone(&metrics), Arc::clone(&peer_streams))
            .expect("first stream");
        let second = StreamMetricGuard::new(Arc::clone(&metrics), Arc::clone(&peer_streams))
            .expect("second stream");
        assert_eq!(peer_streams.load(Ordering::Acquire), 2);
        assert_eq!(observer.snapshot().active_stream_count, 2);

        drop(first);
        assert_eq!(peer_streams.load(Ordering::Acquire), 1);
        assert_eq!(observer.snapshot().active_stream_count, 1);
        drop(second);
        assert_eq!(peer_streams.load(Ordering::Acquire), 0);
        assert_eq!(observer.snapshot().active_stream_count, 0);
    }

    #[tokio::test]
    async fn malformed_and_oversized_first_frames_are_stream_local() {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut malformed = &[0x80, 0x00][..];
        let malformed_error =
            read_one_frame(&mut malformed, zterm_proto::MAX_FRAME_BYTES, deadline)
                .await
                .expect_err("non-canonical frame prefix is malformed");
        assert_eq!(malformed_error.kind(), DomainErrorKind::MalformedFrame);

        let valid_bytes = encode_message(
            WireKind::SessionListRequest,
            1,
            0,
            &v2::SessionListRequest { target: None },
        )
        .expect("bounded service frame");
        let mut oversized = valid_bytes.as_slice();
        let oversized_error = read_one_frame(&mut oversized, 1, deadline)
            .await
            .expect_err("injected one-byte body ceiling rejects the service frame");
        assert_eq!(oversized_error.kind(), DomainErrorKind::FrameTooLarge);

        let mut healthy_peer = valid_bytes.as_slice();
        let frame = read_one_frame(&mut healthy_peer, zterm_proto::MAX_FRAME_BYTES, deadline)
            .await
            .expect("another peer's frame remains readable");
        assert_eq!(frame.kind, WireKind::SessionListRequest);
        assert_eq!(frame.request_id, 1);
    }

    #[tokio::test]
    async fn stalled_first_frame_deadline_is_stream_local() {
        let (_sender, mut stalled) = tokio::io::duplex(64);
        let stalled_error =
            read_one_frame(&mut stalled, zterm_proto::MAX_FRAME_BYTES, Instant::now())
                .await
                .expect_err("elapsed first-frame deadline cannot wait for input");
        assert_eq!(stalled_error.kind(), DomainErrorKind::DeadlineExceeded);

        let healthy_bytes = encode_message(
            WireKind::SessionListRequest,
            2,
            0,
            &v2::SessionListRequest { target: None },
        )
        .expect("bounded service frame");
        let mut healthy_peer = healthy_bytes.as_slice();
        let frame = read_one_frame(
            &mut healthy_peer,
            zterm_proto::MAX_FRAME_BYTES,
            Instant::now() + Duration::from_secs(1),
        )
        .await
        .expect("a stalled peer does not delay another peer");
        assert_eq!(frame.kind, WireKind::SessionListRequest);
        assert_eq!(frame.request_id, 2);
    }

    #[test]
    fn retry_categories_separate_transient_from_terminal_failures() {
        for kind in [
            DomainErrorKind::AddressUnavailable,
            DomainErrorKind::TransportUnavailable,
            DomainErrorKind::DeadlineExceeded,
            DomainErrorKind::ResourceExhausted,
            DomainErrorKind::StoreUnavailable,
        ] {
            assert!(is_retryable(kind), "{} should retry", kind.code());
        }
        for kind in [
            DomainErrorKind::Unauthorized,
            DomainErrorKind::AuthorizationRevoked,
            DomainErrorKind::WireMajorMismatch,
            DomainErrorKind::Cancelled,
            DomainErrorKind::IdentityStateMismatch,
        ] {
            assert!(!is_retryable(kind), "{} must be terminal", kind.code());
        }
    }

    #[test]
    fn pair_tls_identity_mismatch_is_terminal_while_route_failure_can_fall_back() {
        let expected = device(0x51);
        validate_pair_tls_metadata(expected, expected, ZTERM_PAIR_ALPN)
            .expect("exact pair TLS metadata");

        let wrong_id = validate_pair_tls_metadata(expected, device(0x52), ZTERM_PAIR_ALPN)
            .expect_err("ticket host must equal the TLS identity");
        assert_eq!(wrong_id.kind(), DomainErrorKind::Unauthorized);
        assert!(!should_try_next_pair_route(
            &wrong_id,
            Instant::now() + Duration::from_secs(1)
        ));

        let wrong_alpn = validate_pair_tls_metadata(expected, expected, ZTERM_ALPN)
            .expect_err("normal ALPN is never a pair connection");
        assert_eq!(wrong_alpn.kind(), DomainErrorKind::WireMajorMismatch);
        assert!(!should_try_next_pair_route(
            &wrong_alpn,
            Instant::now() + Duration::from_secs(1)
        ));

        let route_failure = transport_unavailable("fixture route failed");
        assert!(should_try_next_pair_route(
            &route_failure,
            Instant::now() + Duration::from_secs(1)
        ));
    }

    #[test]
    fn pair_dial_and_confirmation_source_contracts_are_registry_and_stream_free() {
        let source = include_str!("connection_broker.rs");
        let pair_start = source
            .find("pub(crate) async fn connect_pair_transient")
            .expect("pair dial method");
        let pair_end = source[pair_start..]
            .find("/// Wraps one fully awaited inbound pair connection")
            .map(|offset| pair_start + offset)
            .expect("pair dial method end");
        let pair_body = &source[pair_start..pair_end];
        assert!(pair_body.contains("ZTERM_PAIR_ALPN"));
        assert!(pair_body.contains("endpoint.connect"));
        for forbidden in [
            "peer_slot(",
            "register_candidate(",
            "persist_verified",
            "insert_relay",
        ] {
            assert!(
                !pair_body.contains(forbidden),
                "pair dial must not use normal-registry/profile operation {forbidden}"
            );
        }

        let confirmation_start = source
            .find("async fn wait_for_confirmed_primary")
            .expect("confirmation selection method");
        let confirmation_end = source[confirmation_start..]
            .find("/// Opens a business stream")
            .map(|offset| confirmation_start + offset)
            .expect("confirmation method end");
        let confirmation_body = &source[confirmation_start..confirmation_end];
        assert!(!confirmation_body.contains(concat!(".open", "_bi(")));
        assert!(!source.contains(concat!("into_0", "rtt")));
    }

    #[test]
    fn normal_connections_apply_bi_and_zero_uni_limits_before_handshake() {
        let source = include_str!("connection_broker.rs");
        let accept_start = source
            .find("pub async fn accept_normal")
            .expect("normal accept method");
        let accept_end = source[accept_start..]
            .find("/// Replaces the current Endpoint")
            .map(|offset| accept_start + offset)
            .expect("normal accept method end");
        let accept_body = &source[accept_start..accept_end];
        assert!(
            accept_body
                .find("configure_normal_connection_limits")
                .expect("incoming QUIC limits")
                < accept_body
                    .find("run_inbound_handshake")
                    .expect("incoming application handshake")
        );

        let dial_start = source
            .find("async fn dial_once")
            .expect("normal dial method");
        let dial_end = source[dial_start..]
            .find("async fn dial_routes")
            .map(|offset| dial_start + offset)
            .expect("normal dial method end");
        let dial_body = &source[dial_start..dial_end];
        assert!(
            dial_body
                .find("configure_normal_connection_limits")
                .expect("outgoing QUIC limits")
                < dial_body
                    .find("run_outbound_handshake")
                    .expect("outgoing application handshake")
        );

        let helper_start = source
            .find("fn configure_normal_connection_limits")
            .expect("QUIC limit helper");
        let helper_end = source[helper_start..]
            .find("fn protocol_error")
            .map(|offset| helper_start + offset)
            .expect("QUIC limit helper end");
        let helper_body = &source[helper_start..helper_end];
        assert!(helper_body.contains("set_max_concurrent_bi_streams"));
        assert!(helper_body.contains("set_max_concurrent_uni_streams(VarInt::from_u32(0))"));
    }

    #[tokio::test]
    async fn inbound_admission_is_generic_directional_and_generation_exact() {
        let authorized = device(0x41);
        let revoked = device(0x42);
        let unknown = device(0x43);
        let registry = AuthorizationRegistry::new();
        registry
            .preload(vec![
                authorization(authorized, AuthorizationStatus::Authorized, 1),
                authorization(revoked, AuthorizationStatus::Revoked, 2),
            ])
            .expect("preload registry");

        let admitted =
            admit_inbound_before_payload(&registry, authorized).expect("authorized before payload");
        assert_eq!(
            receiver_generation_for_stream(
                &registry,
                authorized,
                Some(admitted.snapshot.generation),
            )
            .expect("stream uses the receiver-owned accepted generation"),
            admitted.snapshot.generation
        );
        let revoked_error =
            admit_inbound_before_payload(&registry, revoked).expect_err("revoked before payload");
        let unknown_error =
            admit_inbound_before_payload(&registry, unknown).expect_err("unknown before payload");
        assert_eq!(revoked_error.kind(), DomainErrorKind::Unauthorized);
        assert_eq!(unknown_error.kind(), DomainErrorKind::Unauthorized);
        assert_eq!(revoked_error.detail(), unknown_error.detail());

        // Outbound known-device state is deliberately absent from this gate:
        // only receiver-owned device_auth can admit a reverse service stream.
        assert_eq!(
            registry.snapshot(unknown).expect("unknown snapshot"),
            AuthorizationSnapshot::none()
        );

        {
            let mut write = registry
                .authorize_guard(authorized)
                .await
                .expect("authorization writer");
            write
                .publish(AuthorizationSnapshot {
                    status: AuthorizationStatus::Authorized,
                    generation: AuthGeneration::new(2).expect("generation two"),
                })
                .expect("advance generation");
        }
        let stale = recheck_inbound_admission(&registry, authorized, admitted.snapshot)
            .expect_err("generation changed before candidate publication");
        assert_eq!(stale.kind(), DomainErrorKind::Unauthorized);
        let stale_stream = receiver_generation_for_stream(
            &registry,
            authorized,
            Some(admitted.snapshot.generation),
        )
        .expect_err("an old inbound connection cannot adopt a new generation");
        assert_eq!(stale_stream.kind(), DomainErrorKind::Unauthorized);
        assert_eq!(
            receiver_generation_for_stream(&registry, authorized, None)
                .expect("an outbound candidate checks current reverse direction"),
            AuthGeneration::new(2).expect("generation two")
        );
    }

    #[test]
    fn path_projection_never_turns_direct_ip_into_persistable_route() {
        let direct = TransportAddr::Ip("127.0.0.1:4242".parse().expect("socket address"));
        assert_eq!(classify_transport_addr(&direct), (PathKind::Direct, None));

        let relay_url: RelayUrl = "https://relay.example".parse().expect("relay URL parses");
        let relay_path = TransportAddr::Relay(relay_url);
        let (kind, persistable) = classify_transport_addr(&relay_path);
        assert_eq!(kind, PathKind::Relay);
        assert_eq!(
            persistable.as_ref().map(RelayHint::as_str),
            Some("https://relay.example/")
        );
    }
}
