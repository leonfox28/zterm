//! Setup state-machine and identity-stability acceptance tests.

#[path = "support/state_fixture.rs"]
mod state_fixture;

use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

use zterm_core::{DeviceId, DomainErrorKind};
use zterm_daemon::bootstrap::{bootstrap, validate_committed_setup};
use zterm_daemon::config::{ValidatedInfrastructure, validate_setup_input, write_config};
use zterm_daemon::identity::DeviceIdentity;
use zterm_daemon::store::{DeviceMetadata, StateStore};
use zterm_platform::user_state::atomic_write;

use state_fixture::TestState;

fn requested(name: &str) -> zterm_daemon::config::ValidatedConfig {
    validate_setup_input(name, ValidatedInfrastructure::OfficialN0).expect("valid setup input")
}

#[test]
fn first_repeat_and_concurrent_setup_preserve_one_identity() {
    let state = TestState::new();
    let request = requested("workstation");
    let first = bootstrap(&state.paths, &request).expect("first setup");
    let second = bootstrap(&state.paths, &request).expect("repeated setup");
    assert_eq!(first, second);

    let paths = Arc::new(state.paths.clone());
    let handles = (0..8)
        .map(|_| {
            let paths = Arc::clone(&paths);
            let request = request.clone();
            thread::spawn(move || bootstrap(&paths, &request).expect("concurrent setup"))
        })
        .collect::<Vec<_>>();
    for handle in handles {
        assert_eq!(handle.join().expect("setup thread"), first);
    }
    assert_eq!(
        fs::read(paths.identity()).expect("identity bytes").len(),
        32
    );
    assert_eq!(
        validate_committed_setup(&paths).expect("committed setup"),
        first
    );
}

#[test]
fn concurrent_first_setup_serializes_directory_and_identity_creation() {
    let state = TestState::new();
    let paths = Arc::new(state.paths.clone());
    let barrier = Arc::new(Barrier::new(8));
    let handles = (0..8)
        .map(|_| {
            let paths = Arc::clone(&paths);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                bootstrap(&paths, &requested("fresh-concurrent")).expect("concurrent first setup")
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("setup thread"))
        .collect::<Vec<_>>();

    assert!(results.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(fs::read(paths.identity()).expect("one identity").len(), 32);
}

#[test]
fn supported_partial_states_resume_and_missing_key_never_rotates() {
    let key_only = TestState::new();
    key_only
        .paths
        .prepare_state_directories()
        .expect("state dirs");
    let identity = DeviceIdentity::create(&key_only.paths).expect("key only");
    let result = bootstrap(&key_only.paths, &requested("key-only")).expect("key state resumes");
    assert_eq!(result.device_id, identity.device_id());

    let key_config = TestState::new();
    key_config
        .paths
        .prepare_state_directories()
        .expect("state dirs");
    let identity = DeviceIdentity::create(&key_config.paths).expect("identity");
    let request = requested("key-config");
    write_config(&key_config.paths, &request).expect("config");
    assert_eq!(
        bootstrap(&key_config.paths, &request)
            .expect("DB resumes")
            .device_id,
        identity.device_id()
    );

    let key_database = TestState::new();
    key_database
        .paths
        .prepare_state_directories()
        .expect("state dirs");
    let identity = DeviceIdentity::create(&key_database.paths).expect("identity");
    let persisted = requested("key-database");
    let mut store = StateStore::open(&key_database.paths).expect("database");
    store
        .ensure_metadata(&DeviceMetadata {
            device_id: identity.device_id(),
            device_name: persisted.device_name.clone(),
            created_at_unix: 1,
        })
        .expect("matching metadata");
    drop(store);
    let resumed = bootstrap(&key_database.paths, &requested("different-retry-name"))
        .expect("config resumes from database metadata");
    assert_eq!(resumed.device_id, identity.device_id());
    assert_eq!(resumed.config.device_name, persisted.device_name);

    let missing_key = TestState::new();
    missing_key
        .paths
        .prepare_state_directories()
        .expect("state dirs");
    let request = requested("missing-key");
    write_config(&missing_key.paths, &request).expect("orphan config");
    let error = bootstrap(&missing_key.paths, &request).expect_err("missing key rejected");
    assert_eq!(error.kind(), DomainErrorKind::IdentityInvalid);
    assert!(!missing_key.paths.identity().exists());

    let database_without_key = TestState::new();
    database_without_key
        .paths
        .prepare_state_directories()
        .expect("state dirs");
    drop(StateStore::open(&database_without_key.paths).expect("orphan database"));
    let error = bootstrap(&database_without_key.paths, &requested("missing-key"))
        .expect_err("database without key rejected");
    assert_eq!(error.kind(), DomainErrorKind::IdentityInvalid);
    assert!(!database_without_key.paths.identity().exists());
}

#[test]
fn conflicts_bad_key_and_metadata_mismatch_do_not_replace_identity() {
    let state = TestState::new();
    let initial = bootstrap(&state.paths, &requested("original")).expect("setup");
    let bytes = fs::read(state.paths.identity()).expect("identity snapshot");
    let conflict = bootstrap(&state.paths, &requested("different")).expect_err("name conflict");
    assert_eq!(conflict.kind(), DomainErrorKind::AlreadyConfiguredConflict);
    assert_eq!(
        fs::read(state.paths.identity()).expect("identity retained"),
        bytes
    );
    assert_eq!(
        validate_committed_setup(&state.paths)
            .expect("still valid")
            .device_id,
        initial.device_id
    );

    let malformed = TestState::new();
    malformed
        .paths
        .prepare_state_directories()
        .expect("state dirs");
    zterm_platform::user_state::atomic_create(
        malformed.paths.identity(),
        malformed.paths.uid(),
        |file| {
            use std::io::Write;
            file.write_all(b"short")
        },
    )
    .expect("malformed fixture");
    assert_eq!(
        bootstrap(&malformed.paths, &requested("bad"))
            .expect_err("bad key")
            .kind(),
        DomainErrorKind::IdentityInvalid
    );

    let mismatch = TestState::new();
    mismatch
        .paths
        .prepare_state_directories()
        .expect("state dirs");
    let identity = DeviceIdentity::create(&mismatch.paths).expect("identity");
    let mut store = StateStore::open(&mismatch.paths).expect("store");
    store
        .ensure_metadata(&DeviceMetadata {
            device_id: DeviceId::from_array([99; 32]),
            device_name: "mismatch".to_owned(),
            created_at_unix: 1,
        })
        .expect("mismatched fixture metadata");
    drop(store);
    let error = bootstrap(&mismatch.paths, &requested("mismatch")).expect_err("mismatch rejected");
    assert_eq!(error.kind(), DomainErrorKind::IdentityStateMismatch);
    assert_eq!(
        DeviceIdentity::load(&mismatch.paths)
            .expect("identity retained")
            .device_id(),
        identity.device_id()
    );
}

#[test]
fn invalid_committed_config_never_changes_identity_or_database() {
    let cases = [
        ("not = [", DomainErrorKind::ConfigSyntax),
        (
            "schema_version=2\ndevice_name='stable'\n[infrastructure]\nprofile='official-n0'\n",
            DomainErrorKind::ConfigVersion,
        ),
        (
            "schema_version=1\ndevice_name=''\n[infrastructure]\nprofile='official-n0'\n",
            DomainErrorKind::ConfigProfile,
        ),
        (
            "schema_version=1\ndevice_name='stable'\n[infrastructure]\nprofile='official-n0'\nrelay_url='https://relay.example.com'\n",
            DomainErrorKind::ConfigSyntax,
        ),
        (
            "schema_version=1\ndevice_name='stable'\n[infrastructure]\nprofile='self-hosted'\nrelay_url='http://relay.example.com'\n",
            DomainErrorKind::ConfigProfile,
        ),
    ];

    for (raw_config, expected_kind) in cases {
        let state = TestState::new();
        bootstrap(&state.paths, &requested("stable")).expect("initial setup");
        let identity = fs::read(state.paths.identity()).expect("identity snapshot");
        let database = fs::read(state.paths.database()).expect("database snapshot");
        atomic_write(state.paths.config(), state.paths.uid(), |file| {
            use std::io::Write;
            file.write_all(raw_config.as_bytes())
        })
        .expect("invalid config fixture");

        let error = bootstrap(&state.paths, &requested("stable"))
            .expect_err("invalid committed config rejected");
        assert_eq!(error.kind(), expected_kind, "fixture: {raw_config}");
        assert_eq!(
            fs::read(state.paths.identity()).expect("identity retained"),
            identity
        );
        assert_eq!(
            fs::read(state.paths.database()).expect("database retained"),
            database
        );
    }
}
