//! Pure in-memory PairingManager state-machine coverage.
//!
//! This target intentionally creates no Iroh Endpoint, socket, task, or public
//! network dependency. Clock and entropy are injected at the manager boundary.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use zterm_core::{
    AuthGeneration, DeviceDisplayName, DeviceId, DomainErrorKind, EphemeralOperationId,
    PAIR_PROTOCOL_VERSION, PairBegin, PairFingerprint, PairNonce, PairProof, RelayHint,
    TransportLimits,
};
use zterm_daemon::pairing::{
    PairOfferCreated, PairOfferRequest, PairOfferState, PairingClock, PairingClockError,
    PairingEntropy, PairingEntropyError, PairingError, PairingManager, PairingNow,
    PreparedPairChallenge, controller_transcript,
};
use zterm_proto::decode_pair_ticket;

const HOST: DeviceId = DeviceId::from_array([0x11; 32]);
const CONTROLLER_A: DeviceId = DeviceId::from_array([0x22; 32]);
const CONTROLLER_B: DeviceId = DeviceId::from_array([0x33; 32]);

struct FakeClock {
    base: Instant,
    state: Mutex<(u64, Duration)>,
}

impl FakeClock {
    fn new(unix_seconds: u64) -> Self {
        Self {
            base: Instant::now(),
            state: Mutex::new((unix_seconds, Duration::ZERO)),
        }
    }

    fn set_wall(&self, unix_seconds: u64) {
        let mut state = self.state.lock().expect("fake clock lock");
        state.0 = unix_seconds;
    }

    fn advance_wall(&self, seconds: u64) {
        let mut state = self.state.lock().expect("fake clock lock");
        state.0 = state.0.checked_add(seconds).expect("test wall clock fits");
    }

    fn advance_monotonic(&self, duration: Duration) {
        let mut state = self.state.lock().expect("fake clock lock");
        state.1 = state
            .1
            .checked_add(duration)
            .expect("test monotonic clock fits");
    }
}

impl PairingClock for FakeClock {
    fn now(&self) -> Result<PairingNow, PairingClockError> {
        let state = self.state.lock().expect("fake clock lock");
        Ok(PairingNow::new(state.0, self.base + state.1))
    }
}

#[derive(Default)]
struct CountingEntropy {
    calls: AtomicUsize,
}

impl CountingEntropy {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl PairingEntropy for CountingEntropy {
    fn fill(&self, destination: &mut [u8]) -> Result<(), PairingEntropyError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let byte = u8::try_from(call + 1).unwrap_or(0xa5);
        destination.fill(byte);
        Ok(())
    }
}

struct BlockingFirstEntropy {
    calls: AtomicUsize,
    entered: Barrier,
    release: Barrier,
}

impl BlockingFirstEntropy {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            entered: Barrier::new(2),
            release: Barrier::new(2),
        }
    }
}

impl PairingEntropy for BlockingFirstEntropy {
    fn fill(&self, destination: &mut [u8]) -> Result<(), PairingEntropyError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            self.entered.wait();
            self.release.wait();
        }
        let byte = u8::try_from(call + 1).unwrap_or(0x5a);
        destination.fill(byte);
        Ok(())
    }
}

fn limits(max_live_pair_offers: usize) -> TransportLimits {
    TransportLimits {
        max_live_pair_offers,
        pairing_total_deadline: Duration::from_secs(2),
        ..TransportLimits::default()
    }
}

fn test_manager(maximum: usize) -> (PairingManager, Arc<FakeClock>, Arc<CountingEntropy>) {
    let clock = Arc::new(FakeClock::new(1_700_000_000));
    let entropy = Arc::new(CountingEntropy::default());
    let manager =
        PairingManager::with_dependencies(HOST, limits(maximum), clock.clone(), entropy.clone())
            .expect("valid manager");
    (manager, clock, entropy)
}

fn make_request(index: u8) -> PairOfferRequest {
    request_with(index, index)
}

fn request_with(operation: u8, fingerprint: u8) -> PairOfferRequest {
    PairOfferRequest::new(
        EphemeralOperationId::from_array([operation; 16]),
        PairFingerprint::from_bytes([fingerprint; 32]),
        DeviceDisplayName::new("host-device").expect("host name"),
        vec![RelayHint::new("https://relay.example.test.").expect("relay hint")],
        60,
    )
    .expect("valid create request")
}

fn prepared_proof(
    manager: &PairingManager,
    created: &PairOfferCreated,
    controller: DeviceId,
    nonce: u8,
) -> (PreparedPairChallenge, PairProof, [u8; 32]) {
    let (ticket, secret) =
        decode_pair_ticket(created.ticket().expose()).expect("created ticket decodes");
    let begin = PairBegin::new(
        ticket.offer_id(),
        "controller-device",
        PairNonce::from_array([nonce; 32]),
        PAIR_PROTOCOL_VERSION,
    )
    .expect("valid begin");
    let prepared = manager
        .prepare_challenge(controller, &begin)
        .expect("host prepares challenge");
    let offer_key = ticket.offer_key(&secret);
    let proof = PairProof::from_bytes(prepared.transcript().controller_proof(&offer_key));
    (prepared, proof, offer_key)
}

#[test]
fn same_operation_replays_exact_ticket_and_conflicting_fingerprint_is_unknown() {
    let (manager, _, entropy) = test_manager(4);
    let request = make_request(1);

    let first = manager
        .create_offer(request.clone())
        .expect("first create succeeds");
    let second = manager
        .create_offer(request)
        .expect("same operation replays");

    assert_eq!(first.ticket().expose(), second.ticket().expose());
    assert_eq!(first.fields(), second.fields());
    assert_eq!(entropy.calls(), 2, "one offer ID and one secret only");
    assert_eq!(
        manager.create_offer(request_with(1, 9)),
        Err(PairingError::OutcomeUnknown)
    );
    assert_eq!(
        PairingError::OutcomeUnknown.local_kind(),
        DomainErrorKind::PairOutcomeUnknown
    );
}

#[test]
fn concurrent_duplicate_create_joins_the_running_cell_without_second_entropy() {
    let clock = Arc::new(FakeClock::new(1_700_000_000));
    let entropy = Arc::new(BlockingFirstEntropy::new());
    let manager = PairingManager::with_dependencies(HOST, limits(4), clock, entropy.clone())
        .expect("valid manager");
    let request = make_request(2);

    let leader_manager = manager.clone();
    let leader_request = request.clone();
    let leader = thread::spawn(move || leader_manager.create_offer(leader_request));
    entropy.entered.wait();

    let follower_manager = manager.clone();
    let follower = thread::spawn(move || follower_manager.create_offer(request));
    entropy.release.wait();

    let leader = leader
        .join()
        .expect("leader thread")
        .expect("leader result");
    let follower = follower
        .join()
        .expect("follower thread")
        .expect("follower result");
    assert_eq!(leader.ticket().expose(), follower.ticket().expose());
    assert_eq!(
        entropy.calls.load(Ordering::SeqCst),
        2,
        "duplicate join must not generate another offer or secret"
    );
}

#[test]
fn either_wall_or_monotonic_expiry_retires_offer_and_ticket_replay() {
    let (manager, clock, _) = test_manager(4);
    let request = make_request(3);
    let created = manager
        .create_offer(request.clone())
        .expect("offer created");
    clock.advance_wall(60);

    assert_eq!(
        manager.offer_state(created.fields().offer_id()),
        Ok(PairOfferState::Expired)
    );
    assert_eq!(
        manager.create_offer(request),
        Err(PairingError::TicketExpired)
    );
    assert_eq!(manager.snapshot().expect("snapshot").operation_cells, 0);

    let (manager, clock, _) = test_manager(4);
    let created = manager
        .create_offer(make_request(4))
        .expect("second offer created");
    clock.set_wall(1_699_999_000);
    clock.advance_monotonic(Duration::from_secs(60));
    assert_eq!(
        manager.offer_state(created.fields().offer_id()),
        Ok(PairOfferState::Expired),
        "wall-clock rollback must not extend monotonic expiry"
    );
}

#[test]
fn invalid_proof_keeps_ready_only_one_consumer_wins_and_explicit_rollback_reopens() {
    let (manager, _, _) = test_manager(4);
    let created = manager
        .create_offer(make_request(5))
        .expect("offer created");
    let offer_id = created.fields().offer_id();
    let (first, first_proof, _) = prepared_proof(&manager, &created, CONTROLLER_A, 0x41);
    let (second, second_proof, _) = prepared_proof(&manager, &created, CONTROLLER_B, 0x42);

    assert_eq!(
        manager
            .try_consume(first.clone(), &PairProof::from_bytes([0; 32]))
            .expect_err("invalid proof must fail"),
        PairingError::InvalidProof
    );
    assert_eq!(manager.offer_state(offer_id), Ok(PairOfferState::Ready));

    let first = manager
        .try_consume(first, &first_proof)
        .expect("first valid proof wins");
    assert_eq!(manager.offer_state(offer_id), Ok(PairOfferState::Consuming));
    assert_eq!(
        manager
            .try_consume(second.clone(), &second_proof)
            .expect_err("second consumer must lose CAS"),
        PairingError::OfferConsuming
    );
    assert_eq!(manager.rollback(first), Ok(PairOfferState::Ready));

    let second = manager
        .try_consume(second, &second_proof)
        .expect("rollback permits a later valid CAS");
    drop(second);
    assert_eq!(
        manager.offer_state(offer_id),
        Ok(PairOfferState::Consuming),
        "an ambiguous dropped permit must fail closed rather than reopen"
    );
}

#[test]
fn successful_commit_consumes_once_and_confirmation_matches_core_transcript() {
    let (manager, _, _) = test_manager(4);
    let create_request = make_request(6);
    let created = manager
        .create_offer(create_request.clone())
        .expect("offer created");
    let offer_id = created.fields().offer_id();
    let (prepared, proof, offer_key) = prepared_proof(&manager, &created, CONTROLLER_A, 0x51);
    let transcript = prepared.transcript().clone();
    let permit = manager
        .try_consume(prepared.clone(), &proof)
        .expect("valid proof consumes CAS");
    let generation = AuthGeneration::new(7).expect("generation");
    let version = DeviceDisplayName::new("0.1.1").expect("diagnostic version");
    let committed = manager
        .commit(permit, generation, &version)
        .expect("durable commit finalizes offer");

    assert!(transcript.verify_host_confirmation(
        &offer_key,
        generation.get(),
        committed.host_confirmation_proof(),
    ));
    assert_eq!(
        committed
            .pair_accepted()
            .expect("wire projection remains valid")
            .authorization_generation(),
        generation
    );
    let raw_confirmation = format!("{:?}", committed.host_confirmation_proof());
    let confirmation_debug = format!("{committed:?}");
    assert!(confirmation_debug.contains("[REDACTED]"));
    assert!(!confirmation_debug.contains(&raw_confirmation));
    assert_eq!(manager.offer_state(offer_id), Ok(PairOfferState::Consumed));
    assert_eq!(
        manager
            .try_consume(prepared, &proof)
            .expect_err("consumed offer cannot reopen"),
        PairingError::TicketConsumed
    );
    assert_eq!(
        manager.create_offer(create_request),
        Err(PairingError::TicketConsumed),
        "consumption clears the ticket replay and retains a typed tombstone"
    );
}

#[test]
fn rollback_after_expiry_never_reopens_the_offer() {
    let (manager, clock, _) = test_manager(4);
    let created = manager
        .create_offer(make_request(7))
        .expect("offer created");
    let offer_id = created.fields().offer_id();
    let (prepared, proof, _) = prepared_proof(&manager, &created, CONTROLLER_A, 0x61);
    let permit = manager
        .try_consume(prepared, &proof)
        .expect("valid proof owns CAS");
    clock.advance_wall(60);
    clock.advance_monotonic(Duration::from_secs(60));

    assert_eq!(manager.rollback(permit), Ok(PairOfferState::Expired));
    assert_eq!(manager.offer_state(offer_id), Ok(PairOfferState::Expired));
}

#[test]
fn live_offer_and_operation_bounds_are_exact_and_commit_frees_capacity() {
    let (manager, _, _) = test_manager(2);
    let first = manager.create_offer(make_request(8)).expect("first offer");
    manager.create_offer(make_request(9)).expect("second offer");
    assert_eq!(
        manager.create_offer(make_request(10)),
        Err(PairingError::ResourceExhausted)
    );
    let snapshot = manager.snapshot().expect("bounded snapshot");
    assert_eq!(snapshot.ready_offers, 2);
    assert_eq!(snapshot.operation_cells, 2);

    let (prepared, proof, _) = prepared_proof(&manager, &first, CONTROLLER_A, 0x71);
    let permit = manager
        .try_consume(prepared, &proof)
        .expect("first offer consumes");
    manager
        .commit(
            permit,
            AuthGeneration::new(1).expect("generation"),
            &DeviceDisplayName::new("0.1.1").expect("version"),
        )
        .expect("commit");
    manager
        .create_offer(make_request(10))
        .expect("commit immediately frees one live slot");
    let snapshot = manager.snapshot().expect("bounded snapshot");
    assert_eq!(snapshot.ready_offers, 2);
    assert_eq!(snapshot.operation_cells, 2);
    assert_eq!(snapshot.consumed_tombstones, 1);
}

#[test]
fn sequential_create_consume_churn_exceeds_limit_without_growing_or_exhausting() {
    let (manager, _, _) = test_manager(2);

    for (offset, operation) in (20_u8..28).enumerate() {
        let created = manager
            .create_offer(make_request(operation))
            .expect("a completed predecessor must free capacity");
        let (prepared, proof, _) =
            prepared_proof(&manager, &created, CONTROLLER_A, operation.wrapping_add(1));
        let permit = manager
            .try_consume(prepared, &proof)
            .expect("valid proof owns the offer");
        manager
            .commit(
                permit,
                AuthGeneration::new(u64::try_from(offset + 1).expect("small generation"))
                    .expect("generation is in range"),
                &DeviceDisplayName::new("0.1.1").expect("version"),
            )
            .expect("commit retires the operation cell");
        let snapshot = manager.snapshot().expect("bounded snapshot");
        assert_eq!(snapshot.ready_offers, 0);
        assert_eq!(snapshot.operation_cells, 0);
        assert!(snapshot.consumed_tombstones <= 2);
        assert!(snapshot.retired_operation_tombstones <= 2);
    }
}

#[test]
fn controller_transcript_checks_tls_host_and_manager_outputs_are_redacted() {
    let (manager, _, _) = test_manager(4);
    let created = manager
        .create_offer(make_request(11))
        .expect("offer created");
    let (ticket, _) =
        decode_pair_ticket(created.ticket().expose()).expect("created ticket decodes");
    let begin = PairBegin::new(
        ticket.offer_id(),
        "controller-device",
        PairNonce::from_array([0x81; 32]),
        PAIR_PROTOCOL_VERSION,
    )
    .expect("begin");
    let prepared = manager
        .prepare_challenge(CONTROLLER_A, &begin)
        .expect("challenge");
    assert_eq!(
        controller_transcript(
            &ticket,
            DeviceId::from_array([0xff; 32]),
            CONTROLLER_A,
            &begin,
            prepared.challenge(),
        ),
        Err(PairingError::InvalidBinding)
    );

    let ticket_text = created.ticket().expose().to_owned();
    assert_eq!(created.ticket().to_string(), "[REDACTED]");
    assert!(!format!("{created:?}").contains(&ticket_text));
    assert!(!format!("{manager:?}").contains(&ticket_text));
    assert!(!format!("{prepared:?}").contains(&ticket_text));
    let peer = PairingError::TicketConsumed.peer_error();
    assert_eq!(peer.kind(), DomainErrorKind::PairTicketInvalid);
    assert_eq!(peer.detail(), "pairing request was rejected");
    assert!(!peer.to_string().contains(&ticket_text));
}
