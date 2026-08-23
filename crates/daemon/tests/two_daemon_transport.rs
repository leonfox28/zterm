//! Linux-only real-Iroh evidence for two task-private daemon/network owners.
//!
//! Developer workstations compile this target with `cargo test --no-run`.
//! The real loopback case is ignored on macOS before any Endpoint can bind.

use std::time::{Duration, Instant};

use zterm_core::{AuthorizationStatus, TransportLimits};

#[path = "support/two_daemon_fixture.rs"]
mod two_daemon_fixture;
use two_daemon_fixture::PreparedDaemonOwner;

#[test]
fn production_cli_has_no_state_override_argument() {
    let cli = include_str!("../../cli/src/lib.rs");
    let main = include_str!("../../cli/src/main.rs");
    for forbidden in [
        "--state-dir",
        "--state-path",
        "--identity-path",
        "--socket-path",
        "ZTERM_HOME",
        "UserPaths::for_test",
    ] {
        assert!(
            !cli.contains(forbidden) && !main.contains(forbidden),
            "production CLI must not expose task-private state override {forbidden}"
        );
    }
    assert!(cli.contains("internal_daemon"));
    assert!(main.contains("run_internal_daemon"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(target_os = "macos", ignore = "real Iroh loopback is Linux CI only")]
#[cfg_attr(target_os = "windows", ignore = "real Iroh loopback is Linux CI only")]
async fn two_daemon_owners_reuse_endpoint_for_pair_and_normal_confirmation() {
    let relay_a = "https://127.0.0.1:44431"
        .parse()
        .expect("fixture host Relay URL");
    let relay_b = "https://127.0.0.1:44432"
        .parse()
        .expect("fixture controller Relay URL");
    let prepared_a = PreparedDaemonOwner::new("host-a", relay_a);
    let prepared_b = PreparedDaemonOwner::new("controller-b", relay_b);
    let a_id = prepared_a.device_id();
    let b_id = prepared_b.device_id();
    let limits = TransportLimits::default();

    let a = prepared_a
        .bind(&[(b_id, "controller-b")], &[], limits)
        .await;
    let b = prepared_b
        .bind(&[], &[(a_id, "host-a", "host-a")], limits)
        .await;
    let a_sockets = a.bound_sockets();
    let b_sockets = b.bound_sockets();
    assert!(!a_sockets.is_empty());
    assert!(!b_sockets.is_empty());
    assert_ne!(a_id, b_id);

    let profile_before = b.profile_summary();
    assert_eq!(profile_before, b.committed_profile_summary());
    assert_eq!(profile_before.relays.len(), 1);
    assert_eq!(profile_before.relays[0].url, *b.configured_relay());
    assert_ne!(a.configured_relay(), b.configured_relay());

    let before_pair_a = a.broker().observe().snapshot();
    let before_pair_b = b.broker().observe().snapshot();
    assert_eq!(before_pair_a.primary_connection_count, 0);
    assert_eq!(before_pair_b.primary_connection_count, 0);
    let pair_deadline = Instant::now() + Duration::from_secs(15);
    b.probe_pair_connection(&a, pair_deadline)
        .await
        .expect("full pair-ALPN TLS over the controller's sole Endpoint");

    let after_pair_a = a.broker().observe().snapshot();
    let after_pair_b = b.broker().observe().snapshot();
    assert_eq!(after_pair_a.authenticated_connection_count, 0);
    assert_eq!(after_pair_a.primary_connection_count, 0);
    assert_eq!(after_pair_b.authenticated_connection_count, 0);
    assert_eq!(after_pair_b.primary_connection_count, 0);
    assert_eq!(after_pair_b.active_stream_count, 0);
    assert_eq!(a.bound_sockets(), a_sockets);
    assert_eq!(b.bound_sockets(), b_sockets);

    b.broker()
        .set_test_route(a_id, a.direct_address())
        .expect("task-private direct normal route");
    let normal_deadline = Instant::now() + Duration::from_secs(15);
    let transient = b
        .broker()
        .demand_transient(a_id, vec![a.relay_hint()])
        .await
        .expect("transient normal confirmation demand");
    let first_confirmation = transient
        .confirm_authorization(normal_deadline)
        .await
        .expect("normal authorization confirmation");
    assert_eq!(first_confirmation.remote(), a_id);
    assert!(first_confirmation.verified_relay().is_none());
    let first_primary = b
        .broker()
        .peer_observation(a_id)
        .await
        .primary
        .expect("first promoted primary");

    // A durable demand joins the already-promoted connection. Confirmation
    // selects Welcome state only and therefore opens no application stream.
    let durable = b
        .broker()
        .demand(a_id, normal_deadline)
        .await
        .expect("durable known-device demand");
    let second_confirmation = durable
        .confirm_authorization(normal_deadline)
        .await
        .expect("reused normal authorization confirmation");
    assert_eq!(second_confirmation, first_confirmation);
    let second_observation = b.broker().peer_observation(a_id).await;
    assert_eq!(second_observation.primary, Some(first_primary));
    assert_eq!(second_observation.candidate_count, 1);
    assert_eq!(second_observation.active_stream_count, 0);
    a.wait_for_primary(b_id, normal_deadline)
        .await
        .expect("host observes the same normal primary");
    assert_eq!(a.broker().observe().snapshot().primary_connection_count, 1);
    assert_eq!(b.broker().observe().snapshot().primary_connection_count, 1);

    let host_auth = a.authorization_snapshot(b_id);
    assert_eq!(host_auth.status, AuthorizationStatus::Authorized);
    assert_eq!(host_auth.generation, first_confirmation.generation());
    assert_eq!(
        b.authorization_snapshot(a_id).status,
        AuthorizationStatus::None,
        "one-way normal transport must not grant reverse authorization"
    );
    assert!(
        b.known_device(a_id, normal_deadline)
            .expect("controller keeps host in its address book")
            .route_cache
            .is_none(),
        "a transient/direct fixture route must never persist a direct IP"
    );

    let profile_after = b.committed_profile_summary();
    assert_eq!(profile_after, profile_before);
    assert!(
        profile_after
            .relays
            .iter()
            .all(|relay| relay.url != *a.configured_relay()),
        "the remote profile's Relay hint must not enter the configured map"
    );
    for socket in b_sockets {
        assert!(
            !format!("{:?}", b.broker().observe().snapshot()).contains(&socket.to_string()),
            "redacted network observation must not expose a direct address"
        );
    }

    drop(transient);
    drop(durable);
    b.shutdown().await;
    a.shutdown().await;
}
