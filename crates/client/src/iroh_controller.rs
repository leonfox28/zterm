//! Outbound-only Iroh controller used by the mobile application runtime.
use crate::{
    error::ClientError,
    handshake::{ConnectionIdentity, controller_handshake},
    model::ResolvedSessionTarget,
    pairing::{
        ConfirmedAuthorization, ControllerPairAttempt, ControllerPairIdentity, PairProtocolIo,
        resolve_normal_confirmation, run_controller_pair,
    },
    protocol::{
        attachment_cancelled, attachment_command_stream_closed, malformed, protocol_error,
        resource_error,
    },
    remote_unary::{
        RemoteAttemptError, RemoteUnaryClient, RemoteUnaryDemand, RemoteUnaryTransport,
        decode_session_service_error, read_exact_response, timeout_until,
    },
    route::{
        build_relay_candidates, device_from_endpoint_id, endpoint_id_from_device, fresh_relay_hints,
    },
    transport::{
        AttachmentConnector, AttachmentIo, AttachmentTransport, AttachmentTransportItem,
        TransportFuture,
    },
    unary::{SessionUnaryRequest, SessionUnaryTransport},
};
use iroh::{
    Endpoint, RelayMode, SecretKey, TransportAddr,
    endpoint::{Connection, RecvStream, SendStream, VarInt, presets},
};
use ring::rand::{SecureRandom, SystemRandom};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    pin::Pin,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::{Mutex, OwnedSemaphorePermit, Semaphore},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;
use zterm_core::{
    AuthGeneration, Capabilities, ConnectionAttemptId, DeviceId, DomainErrorKind, PairNonce,
    RelayHint, TransportLimits,
};
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, v2};

/// Existing normal service protocol; incoming host services are not exposed.
pub const ZTERM_ALPN: &[u8] = b"zterm/2";
/// Existing one-time pairing protocol.
pub const ZTERM_PAIR_ALPN: &[u8] = b"zterm-pair/2";

/// Builds the official n0 profile with an explicit platform DNS adapter.
/// Publication remains relay-only; authenticated connections can upgrade to direct paths.
pub fn endpoint_builder(secret: SecretKey, dns: iroh::dns::DnsResolver) -> iroh::endpoint::Builder {
    use iroh::address_lookup::{
        AddrFilter, DnsAddressLookup, N0_DNS_ENDPOINT_ORIGIN_PROD, N0_DNS_PKARR_RELAY_PROD,
        PkarrPublisher, PkarrResolver,
    };
    let relay: iroh::RelayUrl = N0_DNS_PKARR_RELAY_PROD
        .parse()
        .expect("pinned n0 production URL");
    Endpoint::builder(presets::Minimal)
        .secret_key(secret)
        .dns_resolver(dns)
        .relay_mode(RelayMode::Default)
        .address_lookup(PkarrPublisher::builder(relay.clone().into()))
        .address_lookup(PkarrResolver::builder(relay.into()))
        .address_lookup(DnsAddressLookup::builder(
            N0_DNS_ENDPOINT_ORIGIN_PROD.to_owned(),
        ))
        .addr_filter(AddrFilter::relay_only())
        .alpns(Vec::new())
}

#[derive(Clone)]
struct NormalConnection {
    connection: Connection,
    generation: AuthGeneration,
    verified_relay: Option<RelayHint>,
}
struct Peer {
    routes: StdMutex<Vec<RelayHint>>,
    normal: Mutex<Option<NormalConnection>>,
    streams: Arc<Semaphore>,
    cancelled: CancellationToken,
}
struct Inner {
    endpoint: Endpoint,
    identity: ConnectionIdentity,
    limits: TransportLimits,
    peers: StdMutex<BTreeMap<DeviceId, Arc<Peer>>>,
    pair_targets: StdMutex<BTreeSet<DeviceId>>,
    dials: Semaphore,
    pairs: Semaphore,
    request_id: AtomicU64,
}

/// Address-free counters for connection health and reproducible acceptance evidence.
#[derive(Clone, Debug)]
pub struct ControllerConnectionInfo {
    /// Selected path class, unknown/direct/relay.
    pub path: &'static str,
    /// Rounded selected path RTT.
    pub rtt_ms: u64,
    /// Transmitted QUIC bytes.
    pub sent_bytes: u64,
    /// Received QUIC bytes.
    pub received_bytes: u64,
    /// Lost packet count.
    pub lost_packets: u64,
    /// Whether the transport has reported closure.
    pub closed: bool,
}

/// One application-owned endpoint and bounded, reused authenticated connections.
#[derive(Clone)]
pub struct IrohController {
    inner: Arc<Inner>,
}

/// Provisional paired identity; commit only after its host record is durably stored.
pub struct PairedHost {
    device_id: DeviceId,
    name: String,
    generation: AuthGeneration,
    verified_relay: Option<RelayHint>,
    peer: Arc<Peer>,
}
impl PairedHost {
    /// TLS-authenticated host public identity.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }
    /// Validated host diagnostic name from the ticket.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Normal Welcome's receiver-owned authorization generation.
    pub fn generation(&self) -> AuthGeneration {
        self.generation
    }
    /// Authenticated route eligible for durable caching.
    pub fn verified_relay(&self) -> Option<&RelayHint> {
        self.verified_relay.as_ref()
    }
}
impl std::fmt::Debug for PairedHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairedHost")
            .field("device_id", &self.device_id)
            .field("name_len", &self.name.len())
            .field("generation", &self.generation)
            .field("route_verified", &self.verified_relay.is_some())
            .finish()
    }
}
impl Peer {
    fn new(routes: Vec<RelayHint>, limits: TransportLimits) -> Self {
        Self {
            routes: StdMutex::new(routes),
            normal: Mutex::new(None),
            streams: Arc::new(Semaphore::new(limits.max_bi_streams_per_connection)),
            cancelled: CancellationToken::new(),
        }
    }
    fn retire(&self) {
        self.cancelled.cancel();
        if let Ok(mut normal) = self.normal.try_lock()
            && let Some(normal) = normal.take()
        {
            normal
                .connection
                .close(VarInt::from_u32(0), b"local host retired");
        }
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.cancelled.cancel();
        if let Some(normal) = self.normal.get_mut().take() {
            normal
                .connection
                .close(VarInt::from_u32(0), b"controller released");
        }
    }
}
struct PairTargetLease {
    inner: Arc<Inner>,
    remote: DeviceId,
}
impl Drop for PairTargetLease {
    fn drop(&mut self) {
        self.inner
            .pair_targets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.remote);
    }
}
impl IrohController {
    /// Composes an already bound platform endpoint without creating host services.
    pub fn new(endpoint: Endpoint, display_name: &str) -> Result<Self, ClientError> {
        let limits = TransportLimits::default();
        let identity = ConnectionIdentity::new(
            device_from_endpoint_id(endpoint.id()),
            display_name,
            env!("CARGO_PKG_VERSION"),
            format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
            Capabilities::from_bits_retain(
                Capabilities::SESSION_SERVICE | Capabilities::TERMINAL_SERVICE,
            ),
        )?;
        Ok(Self {
            inner: Arc::new(Inner {
                endpoint,
                identity,
                limits,
                peers: StdMutex::new(BTreeMap::new()),
                pair_targets: StdMutex::new(BTreeSet::new()),
                dials: Semaphore::new(limits.max_pending_dials),
                pairs: Semaphore::new(limits.max_pairing_handshakes),
                request_id: AtomicU64::new(1),
            }),
        })
    }
    /// Reconciles an OS-reported network change without replacing healthy Sessions.
    pub async fn network_change(&self) {
        self.inner.endpoint.network_change().await;
    }
    /// Public identity of the durable local key used by this endpoint.
    pub fn device_id(&self) -> DeviceId {
        self.inner.identity.device_id()
    }
    /// Closes this controller's endpoint while its network executor is still alive.
    /// Explicit runtime teardown only; foreground/background changes retain it.
    pub async fn shutdown(&self) {
        self.inner.endpoint.close().await;
    }
    /// Observes a retained connection without creating one or probing with input.
    pub async fn connection_info(
        &self,
        remote: DeviceId,
    ) -> Result<Option<ControllerConnectionInfo>, ClientError> {
        let peer = self.peer(remote, None)?;
        let guard = peer.normal.lock().await;
        Ok(guard.as_ref().map(|normal| {
            let stats = normal.connection.stats();
            let (path, rtt_ms) = normal
                .connection
                .paths()
                .iter()
                .find(|path| path.is_selected())
                .map_or(("unknown", 0), |path| {
                    (
                        match path.remote_addr() {
                            TransportAddr::Ip(_) => "direct",
                            TransportAddr::Relay(_) => "relay",
                            _ => "unknown",
                        },
                        u64::try_from(path.rtt().as_millis()).unwrap_or(u64::MAX),
                    )
                });
            ControllerConnectionInfo {
                path,
                rtt_ms,
                sent_bytes: stats.udp_tx.bytes,
                received_bytes: stats.udp_rx.bytes,
                lost_packets: stats.lost_packets,
                closed: normal.connection.close_reason().is_some(),
            }
        }))
    }
    /// Registers a durably known host and its previously authenticated routes.
    pub fn remember_host(
        &self,
        remote: DeviceId,
        routes: Vec<RelayHint>,
    ) -> Result<(), ClientError> {
        self.peer(remote, Some(routes)).map(|_| ())
    }
    /// Publishes a durably saved pair result and reuses its confirmed normal connection.
    pub fn commit_pair(&self, paired: PairedHost) -> Result<(), ClientError> {
        let mut peers = self
            .inner
            .peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !peers.contains_key(&paired.device_id)
            && peers.len() >= self.inner.limits.max_remote_connections
        {
            return Err(resource_error("known connection capacity exhausted"));
        }
        if let Some(old) = peers.insert(paired.device_id, paired.peer) {
            old.retire();
        }
        Ok(())
    }
    /// Retires only this host's local connection state after durable local removal.
    pub async fn forget_host(&self, remote: DeviceId) {
        let peer = self
            .inner
            .peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&remote);
        if let Some(peer) = peer {
            peer.cancelled.cancel();
            if let Some(normal) = peer.normal.lock().await.take() {
                normal
                    .connection
                    .close(VarInt::from_u32(0), b"local host removed");
            }
        }
    }
    fn peer(
        &self,
        remote: DeviceId,
        routes: Option<Vec<RelayHint>>,
    ) -> Result<Arc<Peer>, ClientError> {
        endpoint_id_from_device(remote)?;
        if remote == self.device_id() {
            return Err(ClientError::new(
                DomainErrorKind::PairTicketInvalid,
                "a device cannot pair with itself",
            ));
        }
        if routes
            .as_ref()
            .is_some_and(|routes| routes.len() > self.inner.limits.max_relay_hints)
        {
            return Err(resource_error("host route bound exceeded"));
        }
        let mut peers = self
            .inner
            .peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(peer) = peers.get(&remote) {
            if let Some(routes) = routes {
                *peer
                    .routes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = routes;
            }
            return Ok(Arc::clone(peer));
        }
        let routes = routes.ok_or_else(|| {
            ClientError::new(
                DomainErrorKind::Unauthorized,
                "host is not in the local address book",
            )
        })?;
        if peers.len() >= self.inner.limits.max_remote_connections {
            return Err(resource_error("known connection capacity exhausted"));
        }
        let peer = Arc::new(Peer::new(routes, self.inner.limits));
        peers.insert(remote, Arc::clone(&peer));
        Ok(peer)
    }
    async fn dial(
        &self,
        remote: DeviceId,
        cache: Vec<RelayHint>,
        ticket: Vec<RelayHint>,
        alpn: &[u8],
        deadline: Instant,
    ) -> Result<(Connection, Option<RelayHint>), ClientError> {
        let _permit = timeout_until(deadline, self.inner.dials.acquire())
            .await?
            .map_err(|_| attachment_cancelled())?;
        let endpoint_id = endpoint_id_from_device(remote)?;
        let fresh = fresh_relay_hints(
            self.inner.limits,
            &self.inner.endpoint,
            endpoint_id,
            deadline,
        )
        .await
        .unwrap_or_default();
        let routes = build_relay_candidates(
            endpoint_id,
            fresh,
            cache,
            ticket,
            self.inner.limits.max_relay_hints,
        )?;
        if routes.is_empty() {
            return Err(ClientError::new(
                DomainErrorKind::AddressUnavailable,
                "no relay route is available for the target device",
            ));
        }
        let mut last_error = unavailable("Iroh connect attempt failed");
        for route in routes {
            let until = deadline.min(Instant::now() + self.inner.limits.connect_attempt_budget);
            match timeout_until(
                until,
                self.inner
                    .endpoint
                    .connect(route.endpoint_addr().clone(), alpn),
            )
            .await
            {
                Ok(Ok(connection)) => {
                    if device_from_endpoint_id(connection.remote_id()) != remote {
                        connection.close(VarInt::from_u32(0x101), b"identity mismatch");
                        return Err(ClientError::new(
                            DomainErrorKind::Unauthorized,
                            "Iroh identity does not match the exact target",
                        ));
                    }
                    // The mobile controller accepts no unsolicited host service streams.
                    connection.set_max_concurrent_bi_streams(VarInt::from_u32(0));
                    connection.set_max_concurrent_uni_streams(VarInt::from_u32(0));
                    return Ok((connection, Some(route.relay_hint().clone())));
                }
                Ok(Err(_)) => last_error = unavailable("Iroh connect attempt failed"),
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }
    async fn normal(
        &self,
        remote: DeviceId,
        peer: &Arc<Peer>,
        deadline: Instant,
    ) -> Result<NormalConnection, ClientError> {
        tokio::select! {
            _ = peer.cancelled.cancelled() => Err(attachment_cancelled()),
            result = timeout_until(deadline, async {
                let mut slot = peer.normal.lock().await;
                if let Some(normal) = slot.as_ref().filter(|normal| normal.connection.close_reason().is_none()) { return Ok(normal.clone()); }
                slot.take();
                let routes = peer.routes.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                let (connection, relay) = self.dial(remote, routes, Vec::new(), ZTERM_ALPN, deadline).await?;
                let mut guard = UnconfirmedConnection(Some(connection));
                let connection = guard.0.as_ref().expect("connection guard is populated");
                let welcome = controller_handshake(connection, &self.inner.identity,
                    ConnectionAttemptId::from_array(random_bytes()?),
                    self.inner.limits.max_pair_hello_frame_bytes, deadline).await?;
                let normal = NormalConnection { connection: guard.0.take().expect("validated connection remains present"), generation: welcome.accepted_authorization_generation(), verified_relay: relay };
                *slot = Some(normal.clone());
                Ok(normal)
            }) => result?,
        }
    }
    async fn service_stream(
        &self,
        remote: DeviceId,
        peer: &Arc<Peer>,
        deadline: Instant,
    ) -> Result<IrohSessionIo, ClientError> {
        let normal = self.normal(remote, peer, deadline).await?;
        let permit = timeout_until(deadline, Arc::clone(&peer.streams).acquire_owned())
            .await?
            .map_err(|_| attachment_cancelled())?;
        let (send, recv) = timeout_until(deadline, normal.connection.open_bi())
            .await?
            .map_err(|_| unavailable("unable to open Session stream"))?;
        if peer.cancelled.is_cancelled() {
            return Err(attachment_cancelled());
        }
        Ok(IrohSessionIo {
            connection: normal.connection,
            last_path: None,
            send,
            recv,
            decoder: FrameDecoder::new(),
            queued: VecDeque::new(),
            _permit: permit,
        })
    }
    /// Pairs a validated one-time ticket and resolves ambiguity on the normal ALPN.
    /// The caller must persist the returned host before reporting success to the UI.
    pub async fn pair_ticket(&self, text: &str) -> Result<PairedHost, ClientError> {
        let (fields, secret) = zterm_proto::decode_pair_ticket(text).map_err(|_| {
            ClientError::new(DomainErrorKind::PairTicketInvalid, "invalid pairing ticket")
        })?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| unavailable("pairing clock unavailable"))?
            .as_secs();
        if fields.is_expired(now) {
            return Err(ClientError::new(
                DomainErrorKind::PairTicketExpired,
                "pairing ticket expired",
            ));
        }
        let deadline = Instant::now() + self.inner.limits.pairing_total_deadline;
        let _permit = timeout_until(deadline, self.inner.pairs.acquire())
            .await?
            .map_err(|_| attachment_cancelled())?;
        let remote = fields.host_device_id();
        let routes = fields.relay_hints().to_vec();
        endpoint_id_from_device(remote)?;
        if remote == self.device_id() {
            return Err(ClientError::new(
                DomainErrorKind::PairTicketInvalid,
                "a device cannot pair with itself",
            ));
        }
        if !self
            .inner
            .pair_targets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(remote)
        {
            return Err(resource_error(
                "pairing with this host is already in progress",
            ));
        }
        let _target_lease = PairTargetLease {
            inner: Arc::clone(&self.inner),
            remote,
        };
        let peer = Arc::new(Peer::new(routes.clone(), self.inner.limits));
        let attempt = match self
            .dial(remote, Vec::new(), routes, ZTERM_PAIR_ALPN, deadline)
            .await
        {
            Err(error) => ControllerPairAttempt::failed(error, false),
            Ok((connection, _)) => {
                let result = timeout_until(deadline, connection.open_bi()).await;
                match result {
                    Ok(Ok((send, recv))) => {
                        run_controller_pair(
                            Box::new(IrohPairIo {
                                local: self.device_id(),
                                remote,
                                send,
                                recv,
                                connection,
                            }),
                            &fields,
                            &secret,
                            ControllerPairIdentity {
                                device_id: self.device_id(),
                                display_name: self.inner.identity.display_name(),
                            },
                            &self.inner.limits,
                            deadline,
                            || random_bytes().map(PairNonce::from_array),
                        )
                        .await
                    }
                    Ok(Err(_)) => ControllerPairAttempt::failed(
                        unavailable("unable to open pairing stream"),
                        false,
                    ),
                    Err(error) => ControllerPairAttempt::failed(error, false),
                }
            }
        };
        let confirmation =
            self.normal(remote, &peer, deadline)
                .await
                .map(|normal| ConfirmedAuthorization {
                    remote,
                    generation: normal.generation,
                    verified_relay: normal.verified_relay,
                });
        let confirmation = resolve_normal_confirmation(remote, attempt, confirmation)?;
        Ok(PairedHost {
            device_id: remote,
            name: fields.host_name().to_owned(),
            generation: confirmation.generation,
            verified_relay: confirmation.verified_relay,
            peer,
        })
    }
}
struct UnconfirmedConnection(Option<Connection>);
impl Drop for UnconfirmedConnection {
    fn drop(&mut self) {
        if let Some(connection) = &self.0 {
            connection.close(VarInt::from_u32(0x101), b"handshake incomplete");
        }
    }
}

impl AttachmentConnector for IrohController {
    fn open(
        &self,
        target: ResolvedSessionTarget,
    ) -> TransportFuture<'_, Result<AttachmentTransport, ClientError>> {
        Box::pin(async move {
            let remote = require_remote(target)?;
            let peer = self.peer(remote, None)?;
            let stream = self
                .service_stream(
                    remote,
                    &peer,
                    Instant::now() + crate::protocol::DEFAULT_DEADLINE,
                )
                .await?;
            Ok(AttachmentTransport::new(stream))
        })
    }
}
impl SessionUnaryTransport for IrohController {
    fn request(
        &self,
        request: SessionUnaryRequest,
    ) -> TransportFuture<'_, Result<DecodedFrame, ClientError>> {
        Box::pin(async move {
            let remote = require_remote(request.target)?;
            let request_id = self
                .inner
                .request_id
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| resource_error("request ID exhausted"))?;
            let bytes = Zeroizing::new(
                zterm_proto::encode_payload(
                    request.request_kind,
                    request_id,
                    u32::try_from(request.deadline.as_millis()).unwrap_or(u32::MAX),
                    request.payload,
                )
                .map_err(protocol_error)?,
            );
            RemoteUnaryClient::new(Arc::new(self.clone()))
                .execute_preencoded(
                    remote,
                    request_id,
                    &bytes,
                    Instant::now() + request.deadline,
                )
                .await
                .and_then(session_unary_result)
        })
    }
}
struct IrohDemand {
    controller: IrohController,
    remote: DeviceId,
    peer: Arc<Peer>,
}
// RemoteUnaryClient retains a validated ServiceError frame for forwarding
// adapters. A frontend unary consumer instead needs the typed error, including
// OutcomeUnknown so its old-daemon operation lease can be retired.
fn session_unary_result(frame: DecodedFrame) -> Result<DecodedFrame, ClientError> {
    if frame.kind == WireKind::ServiceErrorResponse {
        Err(decode_session_service_error(&frame)?)
    } else {
        Ok(frame)
    }
}
impl RemoteUnaryTransport for IrohController {
    fn demand<'a>(
        &'a self,
        remote: DeviceId,
        _deadline: Instant,
    ) -> TransportFuture<'a, Result<Box<dyn RemoteUnaryDemand>, ClientError>> {
        Box::pin(async move {
            Ok(Box::new(IrohDemand {
                controller: self.clone(),
                remote,
                peer: self.peer(remote, None)?,
            }) as Box<dyn RemoteUnaryDemand>)
        })
    }
}
impl RemoteUnaryDemand for IrohDemand {
    fn exchange<'a>(
        &'a mut self,
        request: &'a [u8],
        deadline: Instant,
    ) -> TransportFuture<'a, Result<DecodedFrame, RemoteAttemptError>> {
        Box::pin(async move {
            let mut stream = self
                .controller
                .service_stream(self.remote, &self.peer, deadline)
                .await
                .map_err(RemoteAttemptError::PreWrite)?;
            timeout_until(deadline, stream.send.write_all(request))
                .await
                .map_err(RemoteAttemptError::PostWrite)?
                .map_err(|_| {
                    RemoteAttemptError::PostWrite(unavailable("Session request write failed"))
                })?;
            stream.send.finish().map_err(|_| {
                RemoteAttemptError::PostWrite(unavailable("Session request finish failed"))
            })?;
            read_exact_response(&mut stream.recv, deadline).await
        })
    }
}
struct IrohSessionIo {
    connection: Connection,
    last_path: Option<(i32, Option<u32>)>,
    send: SendStream,
    recv: RecvStream,
    decoder: FrameDecoder,
    queued: VecDeque<DecodedFrame>,
    _permit: OwnedSemaphorePermit,
}
impl AttachmentIo for IrohSessionIo {
    fn queued_session_count(&self) -> usize {
        self.queued.len()
    }
    fn write<'a>(&'a mut self, bytes: &'a [u8]) -> TransportFuture<'a, Result<(), ClientError>> {
        Box::pin(async move {
            tokio::io::AsyncWriteExt::write_all(&mut self.send, bytes)
                .await
                .map_err(|error| {
                    if matches!(
                        error.kind(),
                        io::ErrorKind::BrokenPipe
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::NotConnected
                    ) {
                        attachment_command_stream_closed()
                    } else {
                        unavailable("Session input transport failed")
                    }
                })
        })
    }
    fn read(&mut self) -> TransportFuture<'_, Result<AttachmentTransportItem, ClientError>> {
        Box::pin(async move {
            loop {
                let sample = self
                    .connection
                    .paths()
                    .iter()
                    .find(|path| path.is_selected())
                    .map_or((v2::TerminalConnectionPath::Unknown as i32, None), |path| {
                        let kind = match path.remote_addr() {
                            TransportAddr::Ip(_) => v2::TerminalConnectionPath::Direct,
                            TransportAddr::Relay(_) => v2::TerminalConnectionPath::Relay,
                            _ => v2::TerminalConnectionPath::Unknown,
                        };
                        (
                            kind as i32,
                            Some(
                                u32::try_from((path.rtt().as_nanos() + 500_000) / 1_000_000)
                                    .unwrap_or(u32::MAX),
                            ),
                        )
                    });
                if self.last_path != Some(sample) {
                    self.last_path = Some(sample);
                    return Ok(AttachmentTransportItem::Path(v2::LocalSessionTunnelPath {
                        path: sample.0,
                        rtt_ms: sample.1,
                    }));
                }
                if let Some(frame) = self.queued.pop_front() {
                    if !matches!(
                        frame.kind,
                        WireKind::TerminalSemanticSnapshot
                            | WireKind::TerminalSemanticDelta
                            | WireKind::TerminalSemanticHistoryWindowFrame
                            | WireKind::TerminalSyncRequired
                            | WireKind::TerminalLeaseLost
                            | WireKind::TerminalSessionEnded
                            | WireKind::TerminalClipboardWrite
                            | WireKind::SessionOperationLeaseResponse
                            | WireKind::SessionMutateResponse
                            | WireKind::ServiceErrorResponse
                    ) {
                        return Err(malformed(
                            "remote stream carried a non-Session or local-only frame",
                        ));
                    }
                    return Ok(AttachmentTransportItem::Session(frame));
                }
                let mut buffer = Zeroizing::new([0_u8; 16 * 1024]);
                // A quiet shell must still expose path upgrades and new RTT
                // estimates. This only observes QUIC; it sends no terminal input.
                let read = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    self.recv.read(&mut *buffer),
                )
                .await;
                let Ok(read) = read else {
                    continue;
                };
                let count = read
                    .map_err(|_| unavailable("Session stream read failed"))?
                    .unwrap_or(0);
                if count == 0 {
                    std::mem::take(&mut self.decoder)
                        .finish()
                        .map_err(protocol_error)?;
                    return Err(attachment_cancelled());
                }
                self.queued.extend(
                    self.decoder
                        .feed(&buffer[..count])
                        .map_err(protocol_error)?,
                );
            }
        })
    }
    fn shutdown(&mut self) -> TransportFuture<'_, Result<(), ClientError>> {
        Box::pin(async move {
            self.send
                .finish()
                .map_err(|_| unavailable("Session detach finish failed"))
        })
    }
}
impl Drop for IrohSessionIo {
    fn drop(&mut self) {
        let _ = self.recv.stop(VarInt::from_u32(0));
    }
}
struct IrohPairIo {
    local: DeviceId,
    remote: DeviceId,
    send: SendStream,
    recv: RecvStream,
    connection: Connection,
}
impl PairProtocolIo for IrohPairIo {
    fn local(&self) -> DeviceId {
        self.local
    }
    fn remote(&self) -> DeviceId {
        self.remote
    }
}
impl AsyncRead for IrohPairIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        AsyncRead::poll_read(Pin::new(&mut self.get_mut().recv), cx, buf)
    }
}
impl AsyncWrite for IrohPairIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.get_mut().send), cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.get_mut().send), cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.get_mut().send), cx)
    }
}
impl Drop for IrohPairIo {
    fn drop(&mut self) {
        self.connection
            .close(VarInt::from_u32(0x105), b"pair exchange complete");
    }
}
fn require_remote(target: ResolvedSessionTarget) -> Result<DeviceId, ClientError> {
    target.device_id().ok_or_else(|| {
        ClientError::new(
            DomainErrorKind::Unauthorized,
            "mobile controller requires an exact remote host",
        )
    })
}
fn random_bytes<const N: usize>() -> Result<[u8; N], ClientError> {
    let mut bytes = [0; N];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| unavailable("operating-system randomness unavailable"))?;
    Ok(bytes)
}
fn unavailable(detail: &'static str) -> ClientError {
    ClientError::new(DomainErrorKind::TransportUnavailable, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unary::SessionUnaryClient;
    use zterm_core::{DaemonIncarnation, OperationLease, SessionId};

    struct Replies {
        frames: StdMutex<VecDeque<DecodedFrame>>,
        requests: StdMutex<Vec<WireKind>>,
    }
    impl SessionUnaryTransport for Replies {
        fn request(
            &self,
            request: SessionUnaryRequest,
        ) -> TransportFuture<'_, Result<DecodedFrame, ClientError>> {
            self.requests
                .lock()
                .expect("request capture lock")
                .push(request.request_kind);
            let frame = self
                .frames
                .lock()
                .expect("reply fixture lock")
                .pop_front()
                .expect("expected unary request");
            Box::pin(async move { session_unary_result(frame) })
        }
    }
    fn frame(kind: WireKind, message: &impl prost::Message) -> DecodedFrame {
        let bytes =
            zterm_proto::encode_message(kind, 1, 0, message).expect("encode fixture response");
        FrameDecoder::new()
            .feed(&bytes)
            .expect("decode fixture response")
            .pop()
            .expect("one fixture frame")
    }
    fn error(kind: DomainErrorKind) -> DecodedFrame {
        frame(
            WireKind::ServiceErrorResponse,
            &v2::ServiceError {
                code: kind.code().to_owned(),
                message: "PRIVATE_REMOTE_DIAGNOSTIC".to_owned(),
            },
        )
    }
    #[tokio::test]
    async fn restarted_daemon_error_retires_unary_lease_without_replaying_mutation() {
        let lease = |incarnation| {
            frame(
                WireKind::SessionOperationLeaseResponse,
                &v2::SessionOperationLeaseResponse {
                    lease: Some(
                        OperationLease {
                            daemon_incarnation: DaemonIncarnation::from_array([incarnation; 16]),
                            ordinal: 1,
                        }
                        .into(),
                    ),
                },
            )
        };
        let transport = Replies {
            frames: StdMutex::new(VecDeque::from([
                lease(1),
                error(DomainErrorKind::OperationOutcomeUnknown),
                lease(2),
                error(DomainErrorKind::SessionNotFound),
            ])),
            requests: StdMutex::new(Vec::new()),
        };
        let client = SessionUnaryClient::default();
        let target = ResolvedSessionTarget::device(DeviceId::from_array([7; 32]));
        let session = SessionId::from_array([1; 16]);
        let first = client
            .close_session_at(&transport, target, session)
            .await
            .expect_err("definitive fixture service failure");
        assert_eq!(first.kind(), DomainErrorKind::OperationOutcomeUnknown);
        assert!(!first.detail().contains("PRIVATE_REMOTE_DIAGNOSTIC"));
        assert_eq!(
            *transport.requests.lock().expect("request capture lock"),
            vec![
                WireKind::SessionOperationLeaseRequest,
                WireKind::SessionCloseRequest
            ]
        );
        // Only a new explicit call acquires the new daemon's lease. Its definitive
        // service failure remains typed rather than being decoded as Session data.
        let second = client
            .close_session_at(&transport, target, session)
            .await
            .expect_err("definitive fixture service failure");
        assert_eq!(second.kind(), DomainErrorKind::SessionNotFound);
        assert_eq!(
            *transport.requests.lock().expect("request capture lock"),
            vec![
                WireKind::SessionOperationLeaseRequest,
                WireKind::SessionCloseRequest,
                WireKind::SessionOperationLeaseRequest,
                WireKind::SessionCloseRequest
            ]
        );
        for kind in [
            DomainErrorKind::Unauthorized,
            DomainErrorKind::SessionOccupied,
        ] {
            assert_eq!(
                session_unary_result(error(kind))
                    .expect_err("typed service error")
                    .kind(),
                kind
            );
        }
    }
}
