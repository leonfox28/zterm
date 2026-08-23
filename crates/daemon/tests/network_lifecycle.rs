//! Network lifecycle evidence which never opens a socket or contacts a lookup service.

use std::time::{Duration, Instant};

use zterm_core::TransportLimits;
use zterm_daemon::authorization::AuthorizationRegistry;
use zterm_daemon::connection_broker::ConnectionIdentity;
use zterm_daemon::identity::DeviceIdentity;
use zterm_daemon::network::{
    AddressServiceState, NetworkDiagnostic, NetworkObservation, NetworkObserver, NetworkStartup,
    NetworkState, NetworkTestHooks,
};
use zterm_daemon::store::{StateStore, StoreActor};
use zterm_daemon::transport::InfrastructureProfile;

#[path = "support/state_fixture.rs"]
mod state_fixture;
use state_fixture::TestState;

#[test]
fn retry_backoff_and_jitter_are_deterministic_and_capped() {
    let device_id = zterm_core::DeviceId::from_array([0x5a; 32]);
    let hooks = NetworkTestHooks::injected_bind_failures_with_backoff(
        usize::MAX,
        Duration::from_millis(250),
        Duration::from_secs(10),
    );
    let first = (0..12)
        .map(|attempt| hooks.retry_delay_for_test(attempt, device_id))
        .collect::<Vec<_>>();
    let repeated = (0..12)
        .map(|attempt| hooks.retry_delay_for_test(attempt, device_id))
        .collect::<Vec<_>>();

    assert_eq!(
        first, repeated,
        "one identity must get one stable jitter curve"
    );
    assert!(first[0] >= Duration::from_millis(250));
    assert!(first.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(first.iter().all(|delay| *delay <= Duration::from_secs(10)));
    assert!(
        first[6..]
            .iter()
            .all(|delay| *delay == Duration::from_secs(10))
    );
}

#[test]
fn disabled_observation_is_passive_and_truthful() {
    let device_id = zterm_core::DeviceId::from_array([0x2c; 32]);
    let observation = NetworkObserver::disabled(device_id).snapshot();

    assert_eq!(observation.device_id, device_id);
    assert_eq!(observation.state, NetworkState::Disabled);
    assert!(!observation.endpoint_bound);
    assert_eq!(observation.bind_attempts, 0);
    assert_eq!(observation.publish, AddressServiceState::Disabled);
    assert_eq!(observation.lookup, AddressServiceState::Disabled);
    assert_eq!(observation.diagnostic, None);
}

#[tokio::test(flavor = "current_thread")]
async fn injected_failures_never_bind_and_keep_local_owners_responsive() {
    let state = TestState::new();
    state
        .paths
        .prepare_state_directories()
        .expect("isolated state directories");
    let identity = DeviceIdentity::create(&state.paths).expect("isolated identity");
    let device_id = identity.device_id();
    let store = StateStore::open(&state.paths).expect("isolated state store");
    let store = StoreActor::start(store).expect("store owner starts");
    let authorization = AuthorizationRegistry::new();
    let connection_identity = ConnectionIdentity::product(device_id, "network-lifecycle-fixture")
        .expect("connection diagnostics");
    let (startup, handle) = NetworkStartup::prepare(
        identity,
        InfrastructureProfile::zterm(),
        connection_identity,
        store.handle(),
        authorization,
        TransportLimits::default(),
    )
    .expect("network ownership composes without I/O");

    let initial = handle.observe().snapshot();
    assert_initializing(&initial, device_id);

    // Every attempt fails in the hook before Endpoint::builder(...).bind().
    // `usize::MAX` makes accidental fallthrough to a real UDP bind impossible.
    let hooks = NetworkTestHooks::injected_bind_failures_with_backoff(
        usize::MAX,
        Duration::from_millis(1),
        Duration::from_millis(4),
    );
    let close_observer = hooks.clone();
    let mut supervisor = startup.with_test_hooks(hooks).spawn(handle.clone());
    let degraded = wait_for_observation(handle.observe(), |observation| {
        observation.state == NetworkState::Degraded && observation.bind_attempts >= 3
    })
    .await;

    assert_eq!(degraded.device_id, device_id);
    assert!(!degraded.endpoint_bound);
    assert_eq!(degraded.publish, AddressServiceState::Degraded);
    assert_eq!(degraded.lookup, AddressServiceState::Degraded);
    assert_eq!(
        degraded.diagnostic,
        Some(NetworkDiagnostic::EndpointBindFailed)
    );
    assert_eq!(close_observer.endpoint_close_count(), 0);

    // Network retry/degradation is an independent observation: the sole local
    // store owner remains usable while the supervisor is between bind attempts.
    let store_handle = store.handle();
    let metadata = tokio::task::spawn_blocking(move || {
        store_handle.metadata(Instant::now() + Duration::from_secs(1))
    })
    .await
    .expect("local owner task joins")
    .expect("local store remains responsive");
    assert!(metadata.is_none());

    supervisor
        .shutdown_until(Instant::now() + Duration::from_secs(1))
        .await
        .expect("injected pre-bind supervisor stops");
    let stopped = handle.observe().snapshot();
    assert_eq!(stopped.state, NetworkState::Stopped);
    assert!(!stopped.endpoint_bound);
    assert_eq!(stopped.device_id, device_id);
    assert_eq!(close_observer.endpoint_close_count(), 0);

    // Successful shutdown is idempotent and cannot regress the terminal state
    // or attempt an Endpoint close when no Endpoint was ever created.
    supervisor
        .shutdown_until(Instant::now() + Duration::from_secs(1))
        .await
        .expect("second shutdown is idempotent");
    assert_eq!(handle.observe().snapshot(), stopped);
    assert_eq!(close_observer.endpoint_close_count(), 0);
}

fn assert_initializing(observation: &NetworkObservation, device_id: zterm_core::DeviceId) {
    assert_eq!(observation.device_id, device_id);
    assert_eq!(observation.state, NetworkState::Initializing);
    assert!(!observation.endpoint_bound);
    assert_eq!(observation.bind_attempts, 0);
    assert_eq!(observation.publish, AddressServiceState::Degraded);
    assert_eq!(observation.lookup, AddressServiceState::Degraded);
    assert_eq!(observation.diagnostic, None);
}

async fn wait_for_observation(
    observer: NetworkObserver,
    ready: impl Fn(&NetworkObservation) -> bool,
) -> NetworkObservation {
    let mut receiver = observer.subscribe();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        let observation = receiver.borrow().clone();
        if ready(&observation) {
            return observation;
        }
        tokio::time::timeout_at(deadline, receiver.changed())
            .await
            .expect("network observation deadline")
            .expect("network reporter remains live");
    }
}
