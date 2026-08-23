//! Socket-free route ordering and redacted path-observation gate.

use iroh::{RelayUrl, SecretKey, TransportAddr};
use zterm_core::{
    AuthGeneration, AuthorizationSnapshot, AuthorizationStatus, DeviceDisplayName, DeviceId,
    RelayHint,
};
use zterm_daemon::authorization::AuthorizationRegistry;
use zterm_daemon::connection_broker::path_observation_test_evidence;
use zterm_daemon::network::PathKind;
use zterm_daemon::route::{RouteSource, plan_relay_candidates_for_test};
use zterm_daemon::session::SessionService;
use zterm_daemon::store::DeviceAuthorization;
use zterm_daemon::transport::InfrastructureProfile;

fn relay(url: &str) -> RelayHint {
    RelayHint::new(url).expect("fixture Relay URL is valid")
}

#[test]
fn fallback_and_path_changes_leave_auth_session_generation_and_profile_unchanged()
-> Result<(), String> {
    let local = DeviceId::from_array([0x31; 32]);
    let remote_endpoint = SecretKey::from_bytes(&[0x41; 32]).public();
    let remote = DeviceId::from_array(*remote_endpoint.as_bytes());
    let generation = AuthGeneration::new(7).expect("fixture generation is valid");
    let authorization = AuthorizationRegistry::new();
    authorization
        .preload(vec![DeviceAuthorization {
            device_id: remote,
            display_name: DeviceDisplayName::new("remote").expect("fixture display name"),
            status: AuthorizationStatus::Authorized,
            generation,
            paired_at_unix: 1,
            revoked_at_unix: None,
            last_seen_at_unix: None,
        }])
        .map_err(|error| error.to_string())?;
    let authorization_before = authorization
        .snapshot(remote)
        .map_err(|error| error.to_string())?;
    assert_eq!(
        authorization_before,
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation,
        }
    );

    let sessions = SessionService::new(local);
    let sessions_before = sessions.list().map_err(|error| error.to_string())?;
    let profile = InfrastructureProfile::SelfHosted {
        relay_url: "https://home.example"
            .parse()
            .expect("fixture home Relay URL parses"),
    };
    let profile_before = profile.summary();

    let candidates = plan_relay_candidates_for_test(
        remote,
        vec![relay("https://fresh.example")],
        vec![
            relay("https://fresh.example"),
            relay("https://cache.example"),
        ],
        vec![relay("https://ticket.example")],
        4,
    )
    .map_err(|error| error.to_string())?;
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| (candidate.source(), candidate.relay_hint().as_str()))
            .collect::<Vec<_>>(),
        vec![
            (RouteSource::FreshLookup, "https://fresh.example"),
            (RouteSource::VerifiedCache, "https://cache.example"),
            (RouteSource::TransientTicket, "https://ticket.example"),
        ]
    );
    assert!(candidates.iter().all(|candidate| {
        candidate.endpoint_addr().id == remote_endpoint
            && candidate.endpoint_addr().ip_addrs().next().is_none()
            && candidate.endpoint_addr().relay_urls().count() == 1
    }));

    let direct_text = "192.0.2.44:4242";
    let direct = TransportAddr::Ip(direct_text.parse().expect("fixture direct address parses"));
    let relay_url: RelayUrl = "https://selected.example"
        .parse()
        .expect("fixture selected Relay URL parses");
    let paths =
        path_observation_test_evidence(local, remote, &[direct, TransportAddr::Relay(relay_url)]);
    assert_eq!(
        paths.timeline,
        vec![PathKind::Direct, PathKind::Relay, PathKind::Unknown]
    );
    assert_eq!(paths.persistable_relays[0], None);
    assert_eq!(
        paths.persistable_relays[1].as_ref().map(RelayHint::as_str),
        Some("https://selected.example/")
    );
    assert_eq!(paths.selected_observation.direct_path_count, 0);
    assert_eq!(paths.selected_observation.relay_path_count, 1);
    assert_eq!(paths.cleared_observation.direct_path_count, 0);
    assert_eq!(paths.cleared_observation.relay_path_count, 0);
    assert!(
        !format!("{paths:?}").contains(direct_text),
        "redacted path evidence must never retain or expose a direct IP"
    );

    assert_eq!(
        authorization
            .snapshot(remote)
            .map_err(|error| error.to_string())?,
        authorization_before
    );
    assert_eq!(
        sessions.list().map_err(|error| error.to_string())?,
        sessions_before
    );
    assert_eq!(profile.summary(), profile_before);
    sessions.shutdown().map_err(|error| error.to_string())?;
    Ok(())
}
