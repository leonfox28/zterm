//! Pure secret-leakage gates for pairing values and adapters.
//!
//! This target never creates an Endpoint or socket. Fixed byte sentinels are
//! non-production test data used only to prove redaction and persistence
//! boundaries.

#![cfg(unix)]

#[path = "support/state_fixture.rs"]
mod state_fixture;

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;
use zeroize::{Zeroize, Zeroizing};
use zterm_core::{
    AuthGeneration, DeviceAlias, DeviceId, EphemeralOperationId, MAX_TICKET_TEXT_BYTES,
    PAIR_TICKET_FORMAT_VERSION, PairAccepted, PairFingerprint, PairOfferId, PairProof, PairSecret,
    PairTicketFields, RelayHint,
};
use zterm_daemon::pairing::PairTicketText;
use zterm_daemon::pairing_service::LocalPairAcceptInput;
use zterm_daemon::store::{StateStore, database_bytes};

use state_fixture::TestState;

const SECRET_SENTINEL: [u8; 32] = [0xa7; 32];
const PROOF_SENTINEL: [u8; 32] = [0xb8; 32];

#[derive(Clone)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

struct CaptureGuard(Arc<Mutex<Vec<u8>>>);

impl Write for CaptureGuard {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("capture lock")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CaptureWriter {
    type Writer = CaptureGuard;

    fn make_writer(&'writer self) -> Self::Writer {
        CaptureGuard(Arc::clone(&self.0))
    }
}

fn ticket() -> (PairTicketFields, PairSecret, Zeroizing<String>) {
    let fields = PairTicketFields::new(
        PAIR_TICKET_FORMAT_VERSION,
        DeviceId::from_array([0x11; 32]),
        "secret-test-host",
        vec![RelayHint::new("https://relay.example.test").expect("relay")],
        PairOfferId::from_array([0x33; 16]),
        4_000_000_000,
    )
    .expect("ticket fields");
    let secret = PairSecret::from_bytes(SECRET_SENTINEL);
    let text = Zeroizing::new(zterm_proto::encode_pair_ticket(&fields, &secret));
    (fields, secret, text)
}

fn assert_absent(haystack: &[u8], needle: &[u8], context: &str) {
    assert!(
        !haystack
            .windows(needle.len())
            .any(|window| window == needle),
        "{context} exposed a pairing sentinel"
    );
}

#[test]
fn debug_display_error_status_and_tracing_are_redacted() {
    let (_fields, secret, ticket_text) = ticket();
    let proof = PairProof::from_bytes(PROOF_SENTINEL);
    let accepted = PairAccepted::new(
        AuthGeneration::new(7).expect("generation"),
        PROOF_SENTINEL,
        "test-build",
    )
    .expect("PairAccepted");
    let retained_ticket =
        PairTicketText::from_local_response(ticket_text.as_str().to_owned()).expect("ticket text");
    let input = LocalPairAcceptInput::new(
        EphemeralOperationId::from_array([0x44; 16]),
        PairFingerprint::for_accept(ticket_text.as_bytes(), None),
        ticket_text.as_str().to_owned(),
        None,
    );

    let raw_secret = format!("{SECRET_SENTINEL:?}");
    let raw_proof = format!("{PROOF_SENTINEL:?}");
    let rendered = format!(
        "secret={secret:?}/{secret} proof={proof:?} accepted={accepted:?} ticket={retained_ticket:?}/{retained_ticket} input={input:?}"
    );
    assert!(rendered.contains("[REDACTED]"));
    assert!(!rendered.contains(&raw_secret));
    assert!(!rendered.contains(&raw_proof));
    assert!(!rendered.contains(ticket_text.as_str()));

    let invalid = Zeroizing::new("zterm-pair-v1:not-valid-base64".to_owned());
    let invalid_error = PairTicketText::from_local_response(invalid.as_str().to_owned())
        .expect_err("invalid local response");
    let oversized = Zeroizing::new("x".repeat(MAX_TICKET_TEXT_BYTES + 1));
    let oversized_error = PairTicketText::from_local_response(oversized.as_str().to_owned())
        .expect_err("oversized local response");
    let diagnostic =
        format!("{invalid_error:?} {invalid_error} {oversized_error:?} {oversized_error}");
    assert!(!diagnostic.contains(invalid.as_str()));
    assert!(!diagnostic.contains(oversized.as_str()));

    let captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(CaptureWriter(Arc::clone(&captured)))
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(
            ?secret,
            ?proof,
            ?accepted,
            ?retained_ticket,
            ?input,
            "pairing redaction gate"
        );
    });
    let captured = captured.lock().expect("capture lock");
    assert_absent(&captured, raw_secret.as_bytes(), "tracing");
    assert_absent(&captured, raw_proof.as_bytes(), "tracing");
    assert_absent(&captured, ticket_text.as_bytes(), "tracing");

    let panic_context =
        format!("pairing panic context: {secret:?} {proof:?} {accepted:?} {input:?}");
    assert!(!panic_context.contains(&raw_secret));
    assert!(!panic_context.contains(&raw_proof));
    assert!(!panic_context.contains(ticket_text.as_str()));
}

#[test]
fn sqlite_never_contains_ticket_secret_or_proof_sentinels() {
    let (_fields, _secret, mut ticket_text) = ticket();
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let mut store = StateStore::open(&state.paths).expect("state store");
    store
        .authorize_device(DeviceId::from_array([0x22; 32]), "controller", 1)
        .expect("authorize");
    store
        .confirm_known_device(
            DeviceId::from_array([0x11; 32]),
            &DeviceAlias::new("secret-test-host").expect("alias"),
            "secret-test-host",
            None,
        )
        .expect("known device");
    drop(store);

    let bytes = Zeroizing::new(database_bytes(&state.paths).expect("database bytes"));
    assert_absent(&bytes, &SECRET_SENTINEL, "SQLite");
    assert_absent(&bytes, &PROOF_SENTINEL, "SQLite");
    assert_absent(&bytes, ticket_text.as_bytes(), "SQLite");
    ticket_text.zeroize();
}

#[test]
fn fallible_adapters_take_zeroizing_ownership_before_validation() {
    let daemon_source = include_str!("../src/pairing.rs");
    let constructor_start = daemon_source
        .find("pub fn from_local_response(text: String)")
        .expect("PairTicketText constructor");
    let constructor = &daemon_source[constructor_start..constructor_start + 700];
    let zeroizing = constructor
        .find("let text = Zeroizing::new(text);")
        .expect("ticket input enters zeroizing ownership");
    let length_check = constructor
        .find("if text.len()")
        .expect("ticket length validation");
    let decode = constructor
        .find("decode_pair_ticket(&text)")
        .expect("ticket decode validation");
    assert!(zeroizing < length_check && zeroizing < decode);
    assert!(constructor.contains("Ok(Self(text))"));

    let proto_source = include_str!("../../proto/src/lib.rs");
    let proof_start = proto_source
        .find("impl TryFrom<v2::PairProof> for PairProof")
        .expect("PairProof adapter");
    let proof_adapter = &proto_source[proof_start..proof_start + 450];
    assert!(proof_adapter.contains("Zeroizing::new(value.controller_proof)"));
    let accepted_start = proto_source
        .find("impl TryFrom<v2::PairAccepted> for PairAccepted")
        .expect("PairAccepted adapter");
    let accepted_adapter = &proto_source[accepted_start..accepted_start + 900];
    let proof_owner = accepted_adapter
        .find("Zeroizing::new(host_confirmation_proof)")
        .expect("PairAccepted proof owner");
    let generation_validation = accepted_adapter
        .find("AuthGeneration::new(authorization_generation)")
        .expect("PairAccepted generation validation");
    assert!(proof_owner < generation_validation);
    assert!(accepted_adapter.contains("PairAccepted::from_proof"));
}
