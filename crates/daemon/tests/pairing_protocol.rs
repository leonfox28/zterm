//! Pairing protocol acceptance that is safe on developer workstations.
//!
//! These tests deliberately use only the in-memory pairing state machine and
//! task-private SQLite files. They do not construct an Iroh Endpoint, bind a
//! socket, resolve DNS, or contact a relay. Linux-only two-Endpoint coverage is
//! kept as a separate environment gate.

#![cfg(unix)]

#[path = "support/state_fixture.rs"]
mod state_fixture;

use std::time::{Duration, Instant};

use zeroize::Zeroizing;
use zterm_core::{
    AuthorizationSnapshot, AuthorizationStatus, DeviceAlias, DeviceDisplayName, DeviceId,
    DomainErrorKind, EphemeralOperationId, PAIR_PROTOCOL_VERSION, PairBegin, PairFingerprint,
    PairNonce, PairProof, RelayHint, TransportLimits,
};
use zterm_daemon::device_directory::DeviceDirectory;
use zterm_daemon::pairing::{PairOfferRequest, PairOfferState, PairingError, PairingManager};
use zterm_daemon::store::{StateStore, StoreActor};

use state_fixture::TestState;

const HOST: DeviceId = DeviceId::from_array([0x11; 32]);
const CONTROLLER: DeviceId = DeviceId::from_array([0x22; 32]);

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(2)
}

fn offer(manager: &PairingManager, operation: u8) -> zterm_daemon::pairing::PairOfferCreated {
    let ttl = 60;
    manager
        .create_offer(
            PairOfferRequest::new(
                EphemeralOperationId::from_array([operation; 16]),
                PairFingerprint::for_create(ttl),
                DeviceDisplayName::new("host-device").expect("host name"),
                vec![RelayHint::new("https://relay.example.test").expect("relay")],
                ttl,
            )
            .expect("offer request"),
        )
        .expect("pair offer")
}

#[test]
fn proof_before_cas_and_durable_commit_preserve_one_way_rows() {
    let manager = PairingManager::new(HOST, TransportLimits::default()).expect("pairing manager");
    let created = offer(&manager, 1);
    let offer_id = created.fields().offer_id();
    let (fields, secret) =
        zterm_proto::decode_pair_ticket(created.ticket().expose()).expect("ticket decode");
    let begin = PairBegin::new(
        offer_id,
        "controller-device",
        PairNonce::from_array([0x33; 32]),
        PAIR_PROTOCOL_VERSION,
    )
    .expect("PairBegin");
    let prepared = manager
        .prepare_challenge(CONTROLLER, &begin)
        .expect("challenge");

    assert_eq!(
        manager
            .try_consume(prepared.clone(), &PairProof::from_bytes([0; 32]))
            .expect_err("wrong proof"),
        PairingError::InvalidProof
    );
    assert_eq!(manager.offer_state(offer_id), Ok(PairOfferState::Ready));

    let offer_key = Zeroizing::new(fields.offer_key(&secret));
    let proof = PairProof::from_bytes(prepared.transcript().controller_proof(&offer_key));
    let consumption = manager
        .try_consume(prepared, &proof)
        .expect("valid proof wins CAS");
    assert_eq!(manager.offer_state(offer_id), Ok(PairOfferState::Consuming));

    let host_state = TestState::new();
    host_state
        .paths
        .prepare_state_directories()
        .expect("host state dirs");
    let mut host_store = StateStore::open(&host_state.paths).expect("host store");
    let generation = host_store
        .authorize_device(CONTROLLER, "controller-device", 1)
        .expect("durable host authorization");
    manager
        .commit(
            consumption,
            generation,
            &DeviceDisplayName::new("test-build").expect("build"),
        )
        .expect("manager commit after SQLite");
    assert_eq!(manager.offer_state(offer_id), Ok(PairOfferState::Consumed));
    assert_eq!(
        host_store
            .authorization_snapshot(CONTROLLER)
            .expect("host snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation,
        }
    );
    assert!(
        host_store
            .list_known_devices()
            .expect("host known rows")
            .is_empty(),
        "the host must not create an outbound row for its controller"
    );

    let controller_state = TestState::new();
    controller_state
        .paths
        .prepare_state_directories()
        .expect("controller state dirs");
    let mut controller_store = StateStore::open(&controller_state.paths).expect("controller store");
    controller_store
        .confirm_known_device(
            HOST,
            &DeviceAlias::new("host-device").expect("alias"),
            "host-device",
            None,
        )
        .expect("controller known-device commit");
    assert_eq!(
        controller_store
            .authorization_snapshot(HOST)
            .expect("controller auth snapshot"),
        AuthorizationSnapshot::none(),
        "pair accept must not authorize the host in the reverse direction"
    );
    assert_eq!(
        controller_store
            .list_known_devices()
            .expect("controller known rows")
            .len(),
        1
    );

    // A separate reverse ticket is an independent authorization mutation:
    // B now hosts an offer which A consumes, while the original A<-B row and
    // generation remain untouched.
    let reverse_manager =
        PairingManager::new(CONTROLLER, TransportLimits::default()).expect("reverse manager");
    let reverse_created = offer(&reverse_manager, 2);
    let reverse_offer_id = reverse_created.fields().offer_id();
    let (reverse_fields, reverse_secret) =
        zterm_proto::decode_pair_ticket(reverse_created.ticket().expose())
            .expect("reverse ticket decode");
    let reverse_begin = PairBegin::new(
        reverse_offer_id,
        "original-host",
        PairNonce::from_array([0x66; 32]),
        PAIR_PROTOCOL_VERSION,
    )
    .expect("reverse PairBegin");
    let reverse_prepared = reverse_manager
        .prepare_challenge(HOST, &reverse_begin)
        .expect("reverse challenge");
    let reverse_offer_key = Zeroizing::new(reverse_fields.offer_key(&reverse_secret));
    let reverse_proof = PairProof::from_bytes(
        reverse_prepared
            .transcript()
            .controller_proof(&reverse_offer_key),
    );
    let reverse_consumption = reverse_manager
        .try_consume(reverse_prepared, &reverse_proof)
        .expect("reverse proof");
    let reverse_generation = controller_store
        .authorize_device(HOST, "original-host", 2)
        .expect("independent reverse authorization");
    reverse_manager
        .commit(
            reverse_consumption,
            reverse_generation,
            &DeviceDisplayName::new("test-build").expect("build"),
        )
        .expect("reverse manager commit");
    host_store
        .confirm_known_device(
            CONTROLLER,
            &DeviceAlias::new("controller-device").expect("reverse alias"),
            "controller-device",
            None,
        )
        .expect("reverse known-device commit");

    assert_eq!(generation.get(), 1);
    assert_eq!(reverse_generation.get(), 1);
    let revoked_generation = host_store
        .revoke_device(CONTROLLER, 3)
        .expect("revoke original direction");
    assert_eq!(revoked_generation.get(), 2);
    assert_eq!(
        host_store
            .authorization_snapshot(CONTROLLER)
            .expect("revoked A<-B snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Revoked,
            generation: revoked_generation,
        }
    );
    assert_eq!(
        controller_store
            .authorization_snapshot(HOST)
            .expect("independent B<-A snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: reverse_generation,
        },
        "revoking A<-B must not change the independently paired B<-A direction"
    );
    assert_eq!(
        host_store
            .list_known_devices()
            .expect("host reverse known row")
            .len(),
        1
    );
    assert_eq!(
        controller_store
            .list_known_devices()
            .expect("controller original known row")
            .len(),
        1
    );
}

#[test]
fn explicit_alias_conflict_stops_before_transport_boundary() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor =
        StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("store actor");
    let store = actor.handle();
    let alias = DeviceAlias::new("existing-alias").expect("alias");
    store
        .confirm_known_device(
            DeviceId::from_array([0x44; 32]),
            alias.clone(),
            "existing-device",
            None,
            deadline(),
        )
        .expect("seed alias owner");
    let directory = DeviceDirectory::new(store);
    let mut transport_attempted = false;
    let reservation = directory.reserve_selected_alias(
        DeviceId::from_array([0x55; 32]),
        &DeviceDisplayName::new("new-device").expect("name"),
        Some(alias),
        deadline(),
    );
    if reservation.is_ok() {
        transport_attempted = true;
    }

    assert_eq!(
        reservation
            .expect_err("explicit alias must conflict")
            .kind(),
        DomainErrorKind::DeviceAliasConflict
    );
    assert!(!transport_attempted);
}
