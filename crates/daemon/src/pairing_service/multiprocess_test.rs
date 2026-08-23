//! Linux-only multi-process acceptance for the production pairing coordinator.
//!
//! The parent lib-test process self-spawns two exact ignored helper tests. Each
//! helper owns task-private user state, one long-term identity, one StoreActor,
//! one AuthorizationRegistry/DeviceDirectory, one loopback-only Iroh Endpoint,
//! one ConnectionBroker, and the production PairingService. The bearer ticket
//! crosses only a task-private Unix control socket; it is never placed in argv,
//! the environment, a file, or test output.

use std::fs;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use iroh::endpoint::{Incoming, presets};
use iroh::{Endpoint, EndpointAddr, RelayMode};
use tokio::task::{JoinHandle, JoinSet};
use zeroize::{Zeroize, Zeroizing};
use zterm_core::{
    AuthorizationStatus, Capabilities, DeviceAlias, DeviceId, EphemeralOperationId, PairFingerprint,
};
use zterm_platform::user_state::UserPaths;

use super::*;
use crate::identity::DeviceIdentity;
use crate::network::{AddressServiceState, NetworkReporter};
use crate::store::{DeviceMetadata, StateStore, StoreActor};
use crate::transport::{ZTERM_ALPN, ZTERM_PAIR_ALPN};

const CHILD_MODE_ENV: &str = "ZTERM_TEST_MULTIPROCESS_PAIR_CHILD";
const CONTROL_SOCKET_ENV: &str = "ZTERM_TEST_MULTIPROCESS_PAIR_SOCKET";
const HOST_CHILD_TEST: &str = "pairing_service::multiprocess_test::pairing_host_process_entry";
const CONTROLLER_CHILD_TEST: &str =
    "pairing_service::multiprocess_test::pairing_controller_process_entry";
const CONTROL_DEADLINE: Duration = Duration::from_secs(30);
const CHILD_KILL_GRACE: Duration = Duration::from_secs(5);
const PAIR_DEADLINE: Duration = Duration::from_secs(20);
const MAX_CONTROL_PACKET: usize = 32 * 1024;
const HOST_READY: u8 = 1;
const CONTROLLER_READY: u8 = 2;
const ACCEPT_TICKET: u8 = 3;
const CONTROLLER_VERIFIED: u8 = 4;
const VERIFY_HOST: u8 = 5;
const HOST_VERIFIED: u8 = 6;
const SHUTDOWN_CONTROLLER: u8 = 7;

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "real Iroh multi-process pairing is Linux CI only"
)]
fn two_process_production_pairing_service_is_directional_and_reuses_one_endpoint() {
    assert_linux_before_bind();

    let temporary = private_tempdir("zterm-pair-control-");
    let host_socket = temporary.path().join("host.sock");
    let controller_socket = temporary.path().join("controller.sock");
    let host_listener = UnixListener::bind(&host_socket).expect("host control listener");
    let controller_listener =
        UnixListener::bind(&controller_socket).expect("controller control listener");
    fs::set_permissions(&host_socket, fs::Permissions::from_mode(0o600))
        .expect("private host control socket");
    fs::set_permissions(&controller_socket, fs::Permissions::from_mode(0o600))
        .expect("private controller control socket");

    let mut host = ChildGuard::spawn("host", HOST_CHILD_TEST, &host_socket);
    let mut controller = ChildGuard::spawn("controller", CONTROLLER_CHILD_TEST, &controller_socket);
    assert_ne!(
        host.id(),
        controller.id(),
        "owners must be separate processes"
    );
    assert_ne!(host.id(), std::process::id());
    assert_ne!(controller.id(), std::process::id());

    let mut host_control = accept_control(&host_listener, Instant::now() + CONTROL_DEADLINE);
    let mut controller_control =
        accept_control(&controller_listener, Instant::now() + CONTROL_DEADLINE);
    configure_control_stream(&host_control);
    configure_control_stream(&controller_control);

    let host_ready = read_packet(&mut host_control, HOST_READY).expect("host ready packet");
    let (host_id, host_port, mut ticket) = decode_host_ready(&host_ready);
    drop(host_ready);
    let controller_ready =
        read_packet(&mut controller_control, CONTROLLER_READY).expect("controller ready packet");
    let (controller_id, _controller_port) = decode_owner_ready(&controller_ready);
    assert_ne!(host_id, controller_id);

    write_accept_ticket(&mut controller_control, host_id, host_port, &ticket)
        .expect("send bearer ticket over private control socket");
    ticket.zeroize();

    let controller_verified = read_packet(&mut controller_control, CONTROLLER_VERIFIED)
        .expect("controller verification packet");
    let generation = decode_generation(&controller_verified);
    assert!(generation > 0);

    write_verify_host(&mut host_control, controller_id, generation)
        .expect("request durable host verification");
    let host_verified =
        read_packet(&mut host_control, HOST_VERIFIED).expect("host verification packet");
    assert_eq!(decode_generation(&host_verified), generation);
    write_packet(&mut controller_control, SHUTDOWN_CONTROLLER, &[])
        .expect("release controller after host verification");

    drop(host_control);
    drop(controller_control);
    let child_exit_deadline = Instant::now() + CONTROL_DEADLINE;
    let controller_status = controller.finish_until(child_exit_deadline);
    let host_status = host.finish_until(child_exit_deadline);
    assert_child_success("controller", controller_status);
    assert_child_success("host", host_status);
}

/// Exact ignored entrypoint spawned only by the parent test above.
#[test]
#[ignore = "multi-process helper; spawned by its Linux parent gate"]
fn pairing_host_process_entry() {
    if !is_child_mode("host") {
        return;
    }
    assert_linux_before_bind();
    let control = connect_control();
    child_runtime().block_on(run_host_process(control));
}

/// Exact ignored entrypoint spawned only by the parent test above.
#[test]
#[ignore = "multi-process helper; spawned by its Linux parent gate"]
fn pairing_controller_process_entry() {
    if !is_child_mode("controller") {
        return;
    }
    assert_linux_before_bind();
    let control = connect_control();
    child_runtime().block_on(run_controller_process(control));
}

async fn run_host_process(mut control: UnixStream) {
    let owner = ProcessOwner::bind("host-a", "https://relay-host.invalid").await;
    assert_eq!(
        owner.broker.observe().snapshot().primary_connection_count,
        0
    );
    let deadline = Instant::now() + PAIR_DEADLINE;
    let ttl = zterm_core::DEFAULT_PAIR_TTL_SECONDS;
    let created = owner
        .pairing
        .create_until(
            LocalPairCreateInput::new(
                EphemeralOperationId::from_array([0x41; 16]),
                PairFingerprint::for_create(ttl),
                ttl,
            ),
            deadline,
        )
        .expect("production PairingService creates one ticket");
    write_host_ready(
        &mut control,
        owner.device_id,
        owner.loopback_port(),
        created.ticket().expose().as_bytes(),
    )
    .expect("host ready control packet");
    drop(created);

    let verify = read_packet(&mut control, VERIFY_HOST).expect("host verify command");
    let (controller, expected_generation) = decode_verify_host(&verify);
    let expected_generation = AuthGeneration::new(expected_generation)
        .filter(|generation| *generation != AuthGeneration::ZERO)
        .expect("controller generation is valid");
    let deadline = Instant::now() + PAIR_DEADLINE;
    let durable = owner
        .store
        .authorization_snapshot(controller, deadline)
        .expect("durable host authorization");
    assert_eq!(durable.status, AuthorizationStatus::Authorized);
    assert_eq!(durable.generation, expected_generation);
    assert_eq!(
        owner
            .authorization
            .snapshot(controller)
            .expect("published host authorization"),
        durable
    );
    assert!(
        owner
            .store
            .known_device(controller, deadline)
            .expect("host known-device lookup")
            .is_none(),
        "host must not gain reverse outbound trust"
    );
    owner.wait_for_primary(controller, deadline).await;
    assert_eq!(
        owner.broker.observe().snapshot().primary_connection_count,
        1
    );
    owner.assert_endpoint_unchanged();
    write_generation(&mut control, HOST_VERIFIED, durable.generation.get())
        .expect("host verification result");
    owner.shutdown().await;
}

async fn run_controller_process(mut control: UnixStream) {
    let owner = ProcessOwner::bind("controller-b", "https://relay-controller.invalid").await;
    write_owner_ready(
        &mut control,
        CONTROLLER_READY,
        owner.device_id,
        owner.loopback_port(),
    )
    .expect("controller ready control packet");

    let accept = read_packet(&mut control, ACCEPT_TICKET).expect("pair accept command");
    let (host, host_port, mut ticket) = decode_accept_ticket(&accept);
    drop(accept);
    owner
        .broker
        .set_test_route(host, direct_address(host, host_port))
        .expect("task-private direct route");
    let alias = DeviceAlias::new("host-a").expect("controller alias");
    let fingerprint = PairFingerprint::for_accept(&ticket, Some(&alias));
    let ticket_bytes = std::mem::take(&mut *ticket);
    let ticket_text = match String::from_utf8(ticket_bytes) {
        Ok(ticket) => ticket,
        Err(error) => {
            let mut bytes = error.into_bytes();
            bytes.zeroize();
            panic!("private control ticket was not UTF-8");
        }
    };
    let result = owner
        .pairing
        .accept_until(
            LocalPairAcceptInput::new(
                EphemeralOperationId::from_array([0x42; 16]),
                fingerprint,
                ticket_text,
                Some(alias.clone()),
            ),
            Instant::now() + PAIR_DEADLINE,
        )
        .await
        .expect("production PairingService completes real pair and normal confirmation");
    assert_eq!(result.device_id(), host);
    assert_eq!(result.alias(), &alias);
    assert!(result.verified_relay().is_none());

    let deadline = Instant::now() + PAIR_DEADLINE;
    let known = owner
        .store
        .known_device(host, deadline)
        .expect("controller known-device lookup")
        .expect("controller persists host outbound trust");
    assert_eq!(known.local_alias, alias);
    assert!(known.route_cache.is_none());
    assert_eq!(
        owner
            .authorization
            .snapshot(host)
            .expect("controller reverse authorization snapshot")
            .status,
        AuthorizationStatus::None,
        "pairing must not authorize the host to control the controller"
    );
    assert_eq!(
        owner
            .store
            .authorization_snapshot(host, deadline)
            .expect("controller durable reverse authorization")
            .status,
        AuthorizationStatus::None
    );
    owner.wait_for_primary(host, deadline).await;
    let peer = owner.broker.peer_observation(host).await;
    assert_eq!(peer.active_stream_count, 0);
    assert_eq!(
        owner.broker.observe().snapshot().primary_connection_count,
        1
    );
    owner.assert_endpoint_unchanged();
    write_generation(
        &mut control,
        CONTROLLER_VERIFIED,
        result.authorization_generation().get(),
    )
    .expect("controller verification result");
    let shutdown = read_packet(&mut control, SHUTDOWN_CONTROLLER)
        .expect("controller shutdown command after host verification");
    assert!(shutdown.is_empty());
    owner.shutdown().await;
}

struct ProcessOwner {
    _temporary: tempfile::TempDir,
    device_id: DeviceId,
    endpoint: Endpoint,
    broker: ConnectionBroker,
    pairing: PairingService,
    authorization: AuthorizationRegistry,
    store: StoreHandle,
    store_actor: StoreActor,
    accept_task: JoinHandle<()>,
    bound_sockets: Vec<SocketAddr>,
}

impl ProcessOwner {
    async fn bind(name: &str, home_relay: &str) -> Self {
        assert_linux_before_bind();
        let temporary = private_tempdir("zterm-pair-owner-");
        let home = temporary.path().join("home");
        fs::create_dir(&home).expect("task-private account home");
        let paths = UserPaths::for_test(
            nix::unistd::Uid::effective().as_raw(),
            home.clone(),
            home.join(".zterm"),
            temporary.path().join("run"),
        );
        paths
            .prepare_state_directories()
            .expect("task-private state directories");
        let identity = DeviceIdentity::create(&paths).expect("task-private long-term identity");
        let device_id = identity.device_id();
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(identity.into_secret_key())
            .relay_mode(RelayMode::Disabled)
            .alpns(vec![ZTERM_ALPN.to_vec(), ZTERM_PAIR_ALPN.to_vec()])
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .expect("task-private loopback address")
            .bind()
            .await
            .expect("task-private Iroh Endpoint bind");
        assert_eq!(endpoint.id().as_bytes(), device_id.as_bytes());
        let bound_sockets = endpoint.bound_sockets();
        assert_eq!(bound_sockets.len(), 1);

        let mut state = StateStore::open(&paths).expect("task-private state store");
        state
            .ensure_metadata(&DeviceMetadata {
                device_id,
                device_name: name.to_owned(),
                created_at_unix: 1,
            })
            .expect("task-private metadata");
        let authorization_rows = state
            .list_authorizations()
            .expect("empty authorization preload");
        let store_actor = StoreActor::start(state).expect("task-private StoreActor");
        let store = store_actor.handle();
        let authorization = AuthorizationRegistry::new();
        authorization
            .preload(authorization_rows)
            .expect("task-private authorization registry");
        let directory = DeviceDirectory::new(store.clone());
        let limits = TransportLimits::default();
        let connection_identity = ConnectionIdentity::new(
            device_id,
            name,
            "multiprocess-pairing-gate",
            "linux-loopback",
            Capabilities::from_bits_retain(
                Capabilities::SESSION_SERVICE | Capabilities::TERMINAL_SERVICE,
            ),
        )
        .expect("task-private connection identity");
        let (reporter, observer) = NetworkReporter::initializing(device_id);
        reporter.update(|observation| {
            observation.state = NetworkState::Online;
            observation.endpoint_bound = true;
            observation.bind_attempts = 1;
            observation.home_relay = Some(home_relay.to_owned());
            observation.publish = AddressServiceState::Configured;
            observation.lookup = AddressServiceState::Configured;
        });
        let broker = ConnectionBroker::with_reporter(
            connection_identity.clone(),
            store.clone(),
            authorization.clone(),
            limits,
            reporter,
            observer.clone(),
        )
        .expect("task-private connection broker");
        broker
            .attach_endpoint(endpoint.clone())
            .await
            .expect("broker owns the sole Endpoint");
        let manager = PairingManager::new(device_id, limits).expect("production pairing manager");
        let pairing = PairingService::new(
            manager,
            store.clone(),
            authorization.clone(),
            directory,
            broker.clone(),
            observer,
            connection_identity,
            limits,
        )
        .expect("production PairingService composition");
        let accept_task = spawn_accept_router(endpoint.clone(), broker.clone(), pairing.clone());

        Self {
            _temporary: temporary,
            device_id,
            endpoint,
            broker,
            pairing,
            authorization,
            store,
            store_actor,
            accept_task,
            bound_sockets,
        }
    }

    fn loopback_port(&self) -> u16 {
        match self.bound_sockets.as_slice() {
            [SocketAddr::V4(address)] => address.port(),
            _ => panic!("fixture must own exactly one IPv4 loopback socket"),
        }
    }

    fn assert_endpoint_unchanged(&self) {
        assert_eq!(self.endpoint.bound_sockets(), self.bound_sockets);
        assert_eq!(self.endpoint.id().as_bytes(), self.device_id.as_bytes());
    }

    async fn wait_for_primary(&self, remote: DeviceId, deadline: Instant) {
        loop {
            let observation = self.broker.peer_observation(remote).await;
            if observation.primary.is_some() && observation.candidate_count == 1 {
                return;
            }
            assert!(Instant::now() < deadline, "normal primary deadline elapsed");
            tokio::task::yield_now().await;
        }
    }

    async fn shutdown(mut self) {
        self.pairing
            .shutdown_until(Instant::now() + Duration::from_secs(5))
            .await
            .expect("pairing service shutdown");
        self.broker.quiesce().await;
        self.endpoint.close().await;
        self.accept_task.abort();
        let _ = (&mut self.accept_task).await;
        self.store_actor.shutdown();
    }
}

fn spawn_accept_router(
    endpoint: Endpoint,
    broker: ConnectionBroker,
    pairing: PairingService,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut handlers = JoinSet::new();
        while let Some(incoming) = endpoint.accept().await {
            let broker = broker.clone();
            let pairing = pairing.clone();
            handlers.spawn(async move {
                route_incoming(incoming, broker, pairing).await;
            });
            while handlers.try_join_next().is_some() {}
        }
        handlers.abort_all();
        while handlers.join_next().await.is_some() {}
    })
}

async fn route_incoming(incoming: Incoming, broker: ConnectionBroker, pairing: PairingService) {
    let deadline = Instant::now() + PAIR_DEADLINE;
    let Ok(mut accepting) = incoming.accept() else {
        return;
    };
    let Ok(Ok(alpn)) =
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), accepting.alpn()).await
    else {
        return;
    };
    let Ok(Ok(connection)) =
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), accepting).await
    else {
        return;
    };
    if alpn == ZTERM_ALPN {
        let _ = broker.accept_normal(connection).await;
    } else if alpn == ZTERM_PAIR_ALPN {
        let remote = DeviceId::from_array(*connection.remote_id().as_bytes());
        let Ok(admission) = broker
            .pair_handshake_admission()
            .acquire(remote, deadline)
            .await
        else {
            return;
        };
        let Ok(connection) = broker.pair_from_incoming(connection, admission) else {
            return;
        };
        let _ = pairing.accept_pair_connection(connection, deadline).await;
    }
}

fn direct_address(device_id: DeviceId, port: u16) -> EndpointAddr {
    let endpoint_id =
        iroh::EndpointId::from_bytes(device_id.as_bytes()).expect("task-private remote identity");
    EndpointAddr::new(endpoint_id)
        .with_ip_addr(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)))
}

fn child_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("task-private child runtime")
}

fn private_tempdir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .expect("private task root")
}

fn assert_linux_before_bind() {
    if cfg!(target_os = "macos") {
        panic!("real Iroh multi-process pairing is disabled on macOS before Endpoint bind");
    }
    if !cfg!(target_os = "linux") {
        panic!("real Iroh multi-process pairing is Linux CI only");
    }
}

fn is_child_mode(expected: &str) -> bool {
    std::env::var(CHILD_MODE_ENV).is_ok_and(|actual| actual == expected)
}

fn connect_control() -> UnixStream {
    let path = std::env::var_os(CONTROL_SOCKET_ENV)
        .map(PathBuf::from)
        .expect("parent supplied a task-private control socket");
    let stream = UnixStream::connect(path).expect("connect to parent control socket");
    configure_control_stream(&stream);
    stream
}

fn configure_control_stream(stream: &UnixStream) {
    stream
        .set_read_timeout(Some(CONTROL_DEADLINE))
        .expect("control read deadline");
    stream
        .set_write_timeout(Some(CONTROL_DEADLINE))
        .expect("control write deadline");
}

fn accept_control(listener: &UnixListener, deadline: Instant) -> UnixStream {
    listener
        .set_nonblocking(true)
        .expect("nonblocking control listener");
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "child control connection deadline"
                );
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("child control listener failed: {error}"),
        }
    }
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn spawn(role: &str, test_name: &str, socket: &Path) -> Self {
        let child = Command::new(std::env::current_exe().expect("current lib-test executable"))
            .args(["--exact", test_name, "--ignored"])
            .env(CHILD_MODE_ENV, role)
            .env(CONTROL_SOCKET_ENV, socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn exact pairing helper test");
        Self { child: Some(child) }
    }

    fn id(&self) -> u32 {
        self.child.as_ref().expect("live child").id()
    }

    fn finish_until(&mut self, deadline: Instant) -> ExitStatus {
        if let Some(status) = poll_child_until(self.child.as_mut().expect("live child"), deadline)
            .expect("observe pairing helper")
        {
            self.child.take();
            return status;
        }

        let child = self.child.as_mut().expect("live child");
        let _ = child.kill();
        let kill_deadline = Instant::now()
            .checked_add(CHILD_KILL_GRACE)
            .unwrap_or_else(Instant::now);
        let status = poll_child_until(child, kill_deadline)
            .expect("reap killed pairing helper")
            .expect("pairing helper did not exit after kill deadline");
        self.child.take();
        status
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let deadline = Instant::now()
                .checked_add(CHILD_KILL_GRACE)
                .unwrap_or_else(Instant::now);
            let _ = poll_child_until(&mut child, deadline);
        }
    }
}

fn poll_child_until(child: &mut Child, deadline: Instant) -> io::Result<Option<ExitStatus>> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(5).min(deadline.saturating_duration_since(now)));
    }
}

fn assert_child_success(role: &str, status: ExitStatus) {
    assert!(
        status.success(),
        "{role} pairing helper failed with status {} (child output intentionally discarded)",
        status
    );
}

fn write_packet(stream: &mut UnixStream, kind: u8, payload: &[u8]) -> io::Result<()> {
    let length = payload
        .len()
        .checked_add(1)
        .and_then(|length| u32::try_from(length).ok())
        .filter(|length| *length as usize <= MAX_CONTROL_PACKET)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "control packet too large"))?;
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(&[kind])?;
    stream.write_all(payload)?;
    stream.flush()
}

fn read_packet(stream: &mut UnixStream, expected_kind: u8) -> io::Result<Zeroizing<Vec<u8>>> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length)?;
    let length = usize::try_from(u32::from_be_bytes(length))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid control length"))?;
    if !(1..=MAX_CONTROL_PACKET).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid control packet size",
        ));
    }
    let mut kind = [0_u8; 1];
    stream.read_exact(&mut kind)?;
    if kind[0] != expected_kind {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected control packet kind",
        ));
    }
    let mut payload = Zeroizing::new(vec![0_u8; length - 1]);
    stream.read_exact(&mut payload)?;
    Ok(payload)
}

fn write_host_ready(
    stream: &mut UnixStream,
    device_id: DeviceId,
    port: u16,
    ticket: &[u8],
) -> io::Result<()> {
    let ticket_length = u32::try_from(ticket.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "ticket is too large"))?;
    let mut payload = Zeroizing::new(Vec::with_capacity(32 + 2 + 4 + ticket.len()));
    payload.extend_from_slice(device_id.as_bytes());
    payload.extend_from_slice(&port.to_be_bytes());
    payload.extend_from_slice(&ticket_length.to_be_bytes());
    payload.extend_from_slice(ticket);
    write_packet(stream, HOST_READY, &payload)
}

fn decode_host_ready(payload: &[u8]) -> (DeviceId, u16, Zeroizing<Vec<u8>>) {
    let mut cursor = PacketCursor::new(payload);
    let device_id = cursor.device_id();
    let port = cursor.u16();
    let ticket_length = cursor.u32() as usize;
    let ticket = Zeroizing::new(cursor.take(ticket_length).to_vec());
    cursor.finish();
    (device_id, port, ticket)
}

fn write_owner_ready(
    stream: &mut UnixStream,
    kind: u8,
    device_id: DeviceId,
    port: u16,
) -> io::Result<()> {
    let mut payload = Vec::with_capacity(34);
    payload.extend_from_slice(device_id.as_bytes());
    payload.extend_from_slice(&port.to_be_bytes());
    write_packet(stream, kind, &payload)
}

fn decode_owner_ready(payload: &[u8]) -> (DeviceId, u16) {
    let mut cursor = PacketCursor::new(payload);
    let result = (cursor.device_id(), cursor.u16());
    cursor.finish();
    result
}

fn write_accept_ticket(
    stream: &mut UnixStream,
    host: DeviceId,
    port: u16,
    ticket: &[u8],
) -> io::Result<()> {
    let ticket_length = u32::try_from(ticket.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "ticket is too large"))?;
    let mut payload = Zeroizing::new(Vec::with_capacity(32 + 2 + 4 + ticket.len()));
    payload.extend_from_slice(host.as_bytes());
    payload.extend_from_slice(&port.to_be_bytes());
    payload.extend_from_slice(&ticket_length.to_be_bytes());
    payload.extend_from_slice(ticket);
    write_packet(stream, ACCEPT_TICKET, &payload)
}

fn decode_accept_ticket(payload: &[u8]) -> (DeviceId, u16, Zeroizing<Vec<u8>>) {
    decode_host_ready(payload)
}

fn write_verify_host(
    stream: &mut UnixStream,
    controller: DeviceId,
    generation: u64,
) -> io::Result<()> {
    let mut payload = Vec::with_capacity(40);
    payload.extend_from_slice(controller.as_bytes());
    payload.extend_from_slice(&generation.to_be_bytes());
    write_packet(stream, VERIFY_HOST, &payload)
}

fn decode_verify_host(payload: &[u8]) -> (DeviceId, u64) {
    let mut cursor = PacketCursor::new(payload);
    let result = (cursor.device_id(), cursor.u64());
    cursor.finish();
    result
}

fn write_generation(stream: &mut UnixStream, kind: u8, generation: u64) -> io::Result<()> {
    write_packet(stream, kind, &generation.to_be_bytes())
}

fn decode_generation(payload: &[u8]) -> u64 {
    let mut cursor = PacketCursor::new(payload);
    let generation = cursor.u64();
    cursor.finish();
    generation
}

struct PacketCursor<'a> {
    remaining: &'a [u8],
}

impl<'a> PacketCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    fn take(&mut self, length: usize) -> &'a [u8] {
        let (value, remaining) = self
            .remaining
            .split_at_checked(length)
            .expect("well-formed private control packet");
        self.remaining = remaining;
        value
    }

    fn device_id(&mut self) -> DeviceId {
        DeviceId::from_bytes(self.take(32)).expect("control DeviceId")
    }

    fn u16(&mut self) -> u16 {
        u16::from_be_bytes(self.take(2).try_into().expect("control two-byte integer"))
    }

    fn u32(&mut self) -> u32 {
        u32::from_be_bytes(self.take(4).try_into().expect("control four-byte integer"))
    }

    fn u64(&mut self) -> u64 {
        u64::from_be_bytes(self.take(8).try_into().expect("control eight-byte integer"))
    }

    fn finish(self) {
        assert!(
            self.remaining.is_empty(),
            "control packet has trailing bytes"
        );
    }
}
