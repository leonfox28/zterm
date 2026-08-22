//! Configuration profile acceptance tests.

#[path = "support/state_fixture.rs"]
mod state_fixture;

use zterm_daemon::config::{
    ConfigV1, InfrastructureConfig, ValidatedInfrastructure, load_config, validate_setup_input,
    write_config,
};
use zterm_daemon::transport::InfrastructureProfile;

use state_fixture::TestState;

#[test]
fn official_and_self_hosted_profiles_are_mutually_exclusive_and_validated() {
    let official = ConfigV1 {
        schema_version: 1,
        device_name: "work-mac".to_owned(),
        infrastructure: InfrastructureConfig::OfficialN0,
    }
    .validate()
    .expect("official profile");
    assert_eq!(official.infrastructure.profile_name(), "official-n0");

    let self_hosted = ConfigV1 {
        schema_version: 1,
        device_name: "relay-host".to_owned(),
        infrastructure: InfrastructureConfig::SelfHosted {
            relay_url: "https://relay.example.com".to_owned(),
        },
    }
    .validate()
    .expect("self-hosted profile");
    let summary = InfrastructureProfile::from_validated(&self_hosted.infrastructure).summary();
    assert_eq!(summary.relays.len(), 1);
    assert_eq!(summary.relays[0].url.as_str(), "https://relay.example.com/");
    assert!(!summary.relays[0].quic_address_discovery);

    assert!(
        ConfigV1 {
            schema_version: 1,
            device_name: "bad".to_owned(),
            infrastructure: InfrastructureConfig::SelfHosted {
                relay_url: "http://relay.example.com".to_owned(),
            },
        }
        .validate()
        .is_err()
    );
    assert!(toml::from_str::<ConfigV1>(
        "schema_version=1\ndevice_name='mixed'\n[infrastructure]\nprofile='official-n0'\nrelay_url='https://relay.example.com'\n"
    )
    .is_err());
}

#[test]
fn config_round_trip_uses_one_parser_and_preserves_validated_values() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let requested = validate_setup_input("work-linux", ValidatedInfrastructure::OfficialN0)
        .expect("setup input");
    write_config(&state.paths, &requested).expect("config writes");
    assert_eq!(load_config(&state.paths).expect("config loads"), requested);
}
