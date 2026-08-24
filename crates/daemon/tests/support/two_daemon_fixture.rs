//! Task-private two-owner Iroh fixture for the Linux transport gate.
//!
//! Persistent task-private config remains the production `OfficialN0`
//! selection. The runtime fixture is deliberately narrower: relay-disabled
//! loopback/direct transport. It never installs production DNS/Pkarr lookup,
//! never contacts a Relay, never touches the effective user's state, and
//! creates exactly one Endpoint for each prepared daemon owner.

use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

use iroh::endpoint::{Incoming, VarInt, presets};
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey};
use tokio::sync::Notify;
use tokio::task::{JoinHandle, JoinSet};
use zeroize::Zeroizing;
use zterm_core::{Capabilities, DeviceAlias, DeviceId, TransportLimits};
use zterm_daemon::authorization::AuthorizationRegistry;
use zterm_daemon::config::{
    ValidatedInfrastructure, load_config, validate_setup_input, write_config,
};
use zterm_daemon::connection_broker::{ConnectionBroker, ConnectionIdentity};
use zterm_daemon::identity::DeviceIdentity;
use zterm_daemon::store::{DeviceMetadata, KnownDevice, StateStore, StoreActor, StoreHandle};
use zterm_daemon::transport::{
    InfrastructureProfile, InfrastructureProfileSummary, ZTERM_ALPN, ZTERM_PAIR_ALPN,
};

#[path = "state_fixture.rs"]
mod state_fixture;
use state_fixture::TestState;

const FIXTURE_BUILD: &str = "two-daemon-fixture";
const FIXTURE_PLATFORM: &str = "linux-loopback";
const FIXTURE_TIMESTAMP: i64 = 1;
const FIXTURE_TIMEOUT: Duration = Duration::from_secs(20);

/// Persistent task-private state prepared before either network owner binds.
pub struct PreparedDaemonOwner {
    state: TestState,
    device_id: DeviceId,
    name: String,
}

impl PreparedDaemonOwner {
    /// Commits one isolated config and identity without creating an Endpoint.
    pub fn new(name: &str) -> Self {
        let state = TestState::new();
        state
            .paths
            .prepare_state_directories()
            .expect("task-private state directories");
        let config = validate_setup_input(name, ValidatedInfrastructure::OfficialN0)
            .expect("task-private OfficialN0 config");
        write_config(&state.paths, &config).expect("task-private config commit");
        let identity = DeviceIdentity::create(&state.paths).expect("task-private identity commit");
        let device_id = identity.device_id();
        drop(identity);
        Self {
            state,
            device_id,
            name: name.to_owned(),
        }
    }

    /// Public ID derived from this owner's committed identity file.
    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Binds the sole Endpoint after both fixture identities are known.
    pub async fn bind(
        self,
        inbound: &[(DeviceId, &str)],
        known: &[(DeviceId, &str, &str)],
        limits: TransportLimits,
    ) -> DaemonNetworkOwner {
        assert_linux_before_bind();

        let persisted_identity =
            DeviceIdentity::load(&self.state.paths).expect("task-private identity reload");
        assert!(
            persisted_identity.device_id() == self.device_id,
            "persisted fixture identity must match its prepared owner"
        );
        drop(persisted_identity);

        // DeviceIdentity intentionally does not expose its secret outside the
        // daemon crate. This integration fixture reads only its own temporary
        // file, keeps the read buffer zeroizing, and transfers a copy into the
        // one Endpoint below.
        let secret_bytes = Zeroizing::new(
            fs::read(self.state.paths.identity()).expect("task-private identity bytes"),
        );
        let secret_bytes: &[u8; 32] = secret_bytes
            .as_slice()
            .try_into()
            .expect("identity.key is exactly 32 bytes");
        let secret = SecretKey::from_bytes(secret_bytes);
        assert!(
            DeviceId::from_array(*secret.public().as_bytes()) == self.device_id,
            "task-private secret must derive the prepared public identity"
        );

        let committed = load_config(&self.state.paths).expect("task-private committed config");
        assert!(matches!(
            committed.infrastructure,
            ValidatedInfrastructure::OfficialN0
        ));

        // Do not use InfrastructureProfile::endpoint_builder here: this gate
        // is intentionally independent of public DNS/Pkarr, Relay traffic,
        // and port mapping. OfficialN0 is committed state, not runtime relay
        // evidence in this hermetic gate.
        let bind = Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .relay_mode(RelayMode::Disabled)
            .alpns(vec![ZTERM_ALPN.to_vec(), ZTERM_PAIR_ALPN.to_vec()])
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .unwrap_or_else(|_| panic!("task-private loopback bind configuration failed"))
            .bind();
        let endpoint = tokio::time::timeout(FIXTURE_TIMEOUT, bind)
            .await
            .unwrap_or_else(|_| panic!("task-private Endpoint bind deadline elapsed"))
            .unwrap_or_else(|_| panic!("task-private Endpoint bind failed"));
        assert!(
            endpoint.id().as_bytes() == self.device_id.as_bytes(),
            "task-private Endpoint identity must match its prepared owner"
        );

        let mut store = StateStore::open(&self.state.paths).expect("task-private store");
        store
            .ensure_metadata(&DeviceMetadata {
                device_id: self.device_id,
                device_name: self.name.clone(),
                created_at_unix: FIXTURE_TIMESTAMP,
            })
            .expect("task-private metadata");
        for (remote, display_name) in inbound {
            store
                .authorize_device(*remote, display_name, FIXTURE_TIMESTAMP)
                .expect("fixture inbound authorization");
        }
        for (remote, alias, display_name) in known {
            store
                .upsert_known_device(
                    *remote,
                    &DeviceAlias::new((*alias).to_owned()).expect("fixture known alias"),
                    display_name,
                    None,
                )
                .expect("fixture outbound known device");
        }
        let authorization_rows = store
            .list_authorizations()
            .expect("fixture authorization preload");
        let store_actor = StoreActor::start(store).expect("task-private StoreActor");
        let store_handle = store_actor.handle();
        let authorization = AuthorizationRegistry::new();
        authorization
            .preload(authorization_rows)
            .expect("fixture authorization registry");
        let identity = ConnectionIdentity::new(
            self.device_id,
            self.name.clone(),
            FIXTURE_BUILD,
            FIXTURE_PLATFORM,
            Capabilities::from_bits_retain(
                Capabilities::SESSION_SERVICE | Capabilities::TERMINAL_SERVICE,
            ),
        )
        .expect("fixture connection identity");
        let broker = ConnectionBroker::for_test(
            endpoint.clone(),
            identity,
            store_handle.clone(),
            authorization.clone(),
            limits,
        )
        .expect("fixture connection broker");

        let pair_accepts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pair_changed = std::sync::Arc::new(Notify::new());
        let accept_task = spawn_accept_router(
            endpoint.clone(),
            broker.clone(),
            std::sync::Arc::clone(&pair_accepts),
            std::sync::Arc::clone(&pair_changed),
        );

        DaemonNetworkOwner {
            state: self.state,
            device_id: self.device_id,
            endpoint,
            broker,
            authorization,
            store_handle,
            store_actor: Some(store_actor),
            pair_accepts,
            pair_changed,
            accept_task: Some(accept_task),
        }
    }
}

/// One daemon-like owner of task-private state, StoreActor, broker and Endpoint.
pub struct DaemonNetworkOwner {
    state: TestState,
    device_id: DeviceId,
    endpoint: Endpoint,
    broker: ConnectionBroker,
    authorization: AuthorizationRegistry,
    store_handle: StoreHandle,
    store_actor: Option<StoreActor>,
    pair_accepts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pair_changed: std::sync::Arc<Notify>,
    accept_task: Option<JoinHandle<()>>,
}

impl DaemonNetworkOwner {
    pub fn broker(&self) -> &ConnectionBroker {
        &self.broker
    }

    pub fn bound_sockets(&self) -> Vec<SocketAddr> {
        self.endpoint.bound_sockets()
    }

    pub fn committed_profile_summary(&self) -> InfrastructureProfileSummary {
        let committed = load_config(&self.state.paths).expect("task-private committed config");
        InfrastructureProfile::from_validated(&committed.infrastructure).summary()
    }

    pub fn committed_is_official_n0(&self) -> bool {
        matches!(
            load_config(&self.state.paths)
                .expect("task-private committed config")
                .infrastructure,
            ValidatedInfrastructure::OfficialN0
        )
    }

    pub fn authorization_snapshot(&self, remote: DeviceId) -> zterm_core::AuthorizationSnapshot {
        self.authorization
            .snapshot(remote)
            .expect("fixture authorization snapshot")
    }

    pub fn known_device(&self, remote: DeviceId, deadline: Instant) -> Option<KnownDevice> {
        self.store_handle
            .known_device(remote, deadline)
            .expect("fixture known-device read")
    }

    pub fn direct_address(&self) -> EndpointAddr {
        let mut address = EndpointAddr::new(self.endpoint.id());
        for bound in self.endpoint.bound_sockets() {
            let loopback = match bound {
                SocketAddr::V4(socket) => {
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), socket.port())
                }
                SocketAddr::V6(socket) => {
                    SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), socket.port())
                }
            };
            address = address.with_ip_addr(loopback);
        }
        address
    }

    /// Completes mutual TLS on pair ALPN using this owner's sole Endpoint.
    pub async fn probe_pair_connection(
        &self,
        remote: &Self,
        deadline: Instant,
    ) -> Result<(), String> {
        let address = remote.direct_address();
        let connection = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.endpoint.connect(address, ZTERM_PAIR_ALPN),
        )
        .await
        .map_err(|_| "pair TLS deadline elapsed".to_owned())?
        .map_err(|_| "pair TLS connection failed".to_owned())?;
        if connection.remote_id().as_bytes() != remote.device_id.as_bytes() {
            return Err("pair TLS authenticated an unexpected remote ID".to_owned());
        }
        if connection.alpn() != ZTERM_PAIR_ALPN {
            return Err("pair TLS negotiated an unexpected ALPN".to_owned());
        }
        remote.wait_for_pair_accept(1, deadline).await?;
        connection.close(VarInt::from_u32(0x105), b"pair probe complete");
        Ok(())
    }

    pub async fn wait_for_primary(
        &self,
        remote: DeviceId,
        deadline: Instant,
    ) -> Result<(), String> {
        loop {
            let observation = self.broker.peer_observation(remote).await;
            if observation.primary.is_some() && observation.candidate_count == 1 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("normal primary deadline elapsed".to_owned());
            }
            tokio::task::yield_now().await;
        }
    }

    async fn wait_for_pair_accept(&self, expected: usize, deadline: Instant) -> Result<(), String> {
        loop {
            let notified = self.pair_changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.pair_accepts.load(std::sync::atomic::Ordering::Acquire) >= expected {
                return Ok(());
            }
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), notified)
                .await
                .map_err(|_| "pair accept deadline elapsed".to_owned())?;
        }
    }

    pub async fn shutdown(mut self) {
        let deadline = Instant::now() + FIXTURE_TIMEOUT;
        tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.broker.quiesce(),
        )
        .await
        .expect("broker cleanup deadline");
        tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.endpoint.close(),
        )
        .await
        .expect("Endpoint cleanup deadline");
        if let Some(mut task) = self.accept_task.take() {
            task.abort();
            let _ = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), &mut task)
                .await
                .expect("accept-router cleanup deadline");
        }
        if let Some(actor) = self.store_actor.take() {
            tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                tokio::task::spawn_blocking(move || actor.shutdown()),
            )
            .await
            .expect("StoreActor cleanup deadline")
            .expect("StoreActor cleanup worker");
        }
    }
}

impl Drop for DaemonNetworkOwner {
    fn drop(&mut self) {
        if let Some(task) = self.accept_task.take() {
            task.abort();
        }
    }
}

fn spawn_accept_router(
    endpoint: Endpoint,
    broker: ConnectionBroker,
    pair_accepts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pair_changed: std::sync::Arc<Notify>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut handlers = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                _joined = handlers.join_next(), if !handlers.is_empty() => {}
                incoming = endpoint.accept() => {
                    let Some(incoming) = incoming else {
                        break;
                    };
                    let broker = broker.clone();
                    let pair_accepts = std::sync::Arc::clone(&pair_accepts);
                    let pair_changed = std::sync::Arc::clone(&pair_changed);
                    handlers.spawn(async move {
                        route_incoming(incoming, broker, pair_accepts, pair_changed).await;
                    });
                }
            }
        }
        handlers.abort_all();
        while handlers.join_next().await.is_some() {}
    })
}

async fn route_incoming(
    incoming: Incoming,
    broker: ConnectionBroker,
    pair_accepts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pair_changed: std::sync::Arc<Notify>,
) {
    let Ok(mut accepting) = incoming.accept() else {
        return;
    };
    let Ok(Ok(alpn)) = tokio::time::timeout(Duration::from_secs(10), accepting.alpn()).await else {
        return;
    };
    if alpn == ZTERM_ALPN {
        if let Ok(Ok(connection)) = tokio::time::timeout(Duration::from_secs(10), accepting).await {
            let _ = broker.accept_normal(connection).await;
        }
    } else if alpn == ZTERM_PAIR_ALPN
        && let Ok(Ok(connection)) = tokio::time::timeout(Duration::from_secs(10), accepting).await
    {
        pair_accepts.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        pair_changed.notify_waiters();
        connection.close(VarInt::from_u32(0x105), b"pair probe complete");
    }
}

fn assert_linux_before_bind() {
    #[cfg(not(target_os = "linux"))]
    panic!("real Iroh loopback helpers must fail before bind outside Linux");
}
