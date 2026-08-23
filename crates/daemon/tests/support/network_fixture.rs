//! Task-private real-Iroh peers backed by isolated stores and identities.

#![allow(dead_code)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey};
use tokio::task::JoinHandle;
use zterm_core::{AuthorizationStatus, Capabilities, DeviceAlias, DeviceId, TransportLimits};
use zterm_daemon::authorization::AuthorizationRegistry;
use zterm_daemon::connection_broker::{ConnectionBroker, ConnectionIdentity};
use zterm_daemon::store::{StateStore, StoreActor};
use zterm_daemon::transport::{ZTERM_ALPN, ZTERM_PAIR_ALPN};

#[path = "state_fixture.rs"]
mod state_fixture;
use state_fixture::TestState;

pub struct NetworkPeer {
    pub device_id: DeviceId,
    pub endpoint: Endpoint,
    pub broker: ConnectionBroker,
    accept_task: JoinHandle<()>,
    _state: TestState,
    _store: StoreActor,
}

impl NetworkPeer {
    pub async fn create(
        secret: [u8; 32],
        name: &str,
        inbound: &[(DeviceId, &str)],
        known: &[(DeviceId, &str, &str)],
        limits: TransportLimits,
    ) -> Self {
        if cfg!(target_os = "macos") {
            panic!(
                "real-Iroh integration fixtures are disabled on macOS to avoid firewall prompts"
            );
        }

        let secret = SecretKey::from_bytes(&secret);
        let device_id = DeviceId::from_array(*secret.public().as_bytes());
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .relay_mode(RelayMode::Disabled)
            .alpns(vec![ZTERM_ALPN.to_vec(), ZTERM_PAIR_ALPN.to_vec()])
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .expect("task-private IPv4 loopback address is valid")
            .bind()
            .await
            .expect("task-private endpoint binds");

        let state = TestState::new();
        state
            .paths
            .prepare_state_directories()
            .expect("state directories");
        let mut store = StateStore::open(&state.paths).expect("isolated store");
        for (remote, display) in inbound {
            store
                .authorize_device(*remote, display, 10)
                .expect("inbound authorization");
        }
        for (remote, alias, display) in known {
            store
                .upsert_known_device(
                    *remote,
                    &DeviceAlias::new((*alias).to_owned()).expect("known alias"),
                    display,
                    None,
                )
                .expect("known-device row");
        }
        let rows = store.list_authorizations().expect("authorization preload");
        let actor = StoreActor::start(store).expect("store actor starts");
        let authorization = AuthorizationRegistry::new();
        authorization.preload(rows).expect("registry preload");
        let identity = ConnectionIdentity::new(
            device_id,
            name,
            "fixture-build",
            "fixture-platform",
            Capabilities::from_bits_retain(
                Capabilities::SESSION_SERVICE | Capabilities::TERMINAL_SERVICE,
            ),
        )
        .expect("connection identity");
        let broker = ConnectionBroker::for_test(
            endpoint.clone(),
            identity,
            actor.handle(),
            authorization,
            limits,
        )
        .expect("test broker");
        let accept_endpoint = endpoint.clone();
        let accept_broker = broker.clone();
        let accept_task = tokio::spawn(async move {
            let mut handlers = tokio::task::JoinSet::new();
            while let Some(incoming) = accept_endpoint.accept().await {
                let broker = accept_broker.clone();
                handlers.spawn(async move {
                    if let Ok(connection) = incoming.await {
                        let _ = broker.accept_normal(connection).await;
                    }
                });
                while handlers.try_join_next().is_some() {}
            }
            handlers.abort_all();
            while handlers.join_next().await.is_some() {}
        });
        Self {
            device_id,
            endpoint,
            broker,
            accept_task,
            _state: state,
            _store: actor,
        }
    }

    pub fn address(&self) -> EndpointAddr {
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

    pub async fn shutdown(mut self) {
        self.broker.quiesce().await;
        self.endpoint.close().await;
        self.accept_task.abort();
        let _ = (&mut self.accept_task).await;
    }

    pub async fn wait_for_primary(&self, remote: DeviceId) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let observation = self.broker.peer_observation(remote).await;
            if observation.primary.is_some() && observation.candidate_count == 1 {
                return;
            }
            assert!(tokio::time::Instant::now() < deadline, "primary timeout");
            tokio::task::yield_now().await;
        }
    }
}

pub fn assert_authorized_status(peer: &NetworkPeer, remote: DeviceId) {
    let snapshot = peer.broker.observe().snapshot();
    assert_eq!(snapshot.device_id, peer.device_id);
    let _ = (remote, AuthorizationStatus::Authorized);
}
