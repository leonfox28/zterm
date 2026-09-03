//! Wire compatibility, ticket text, and kind-registry regression tests.

use prost::Message;
use zterm_core::{
    AuthGeneration, Capabilities, ConnectionAttemptId, ConnectionHello, ConnectionWelcome,
    DeviceAlias, DeviceId, DeviceSummary, EphemeralOperationId, PAIR_FINGERPRINT_BYTES,
    PAIR_PROTOCOL_VERSION, PAIR_TICKET_FORMAT_VERSION, PairAccepted, PairBegin, PairChallenge,
    PairFingerprint, PairFingerprintError, PairHandshakeBudget, PairNonce, PairOfferId, PairProof,
    PairSecret, PairTicketFields, RelayHint,
};
use zterm_proto::{
    DecodedFrame, MAX_PAIR_HELLO_FRAME_BYTES, PAIR_TICKET_PREFIX, RELAY_ROUTE_CACHE_VERSION,
    WIRE_MAJOR, WireFieldError, WireKind, decode_pair_ticket, decode_relay_route_cache,
    encode_pair_ticket, encode_payload, encode_relay_route_cache, v1, validate_pair_operation,
};

// Cross-language golden vector from `zterm-core/tests/pairing_vectors.rs`, fixed
// non-production credentials.
const HOST_DEVICE: [u8; 32] = [0x11; 32];
const OFFER_ID: [u8; 16] = [0xaa; 16];
const SECRET: [u8; 32] = [0x42; 32];
const EXPIRES_AT_UNIX: u64 = 1_700_000_000;
const TICKET_CANONICAL_HEX: &str = "7a7465726d2d706169722d7469636b65742d7631000000000111111111111111111111111111111111111111111111111111111111111111110009746573742d686f737401001968747470733a2f2f72656c61792e6578616d706c652e636f6daaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa000000006553f100";
const TICKET_DIGEST_HEX: &str = "3c624fb8cadeba27c9512fa4bcf99e0563f75609a3d0b7d0c013867eefe447fd";

fn hex(input: &str) -> Vec<u8> {
    input
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex chunk is ASCII");
            u8::from_str_radix(text, 16).expect("valid hex digit")
        })
        .collect()
}

fn assert_message_round_trip<MessageType>(message: &MessageType)
where
    MessageType: Message + Default + PartialEq + std::fmt::Debug,
{
    let bytes = message.encode_to_vec();
    let decoded = MessageType::decode(bytes.as_slice()).expect("generated message round trips");
    assert_eq!(&decoded, message);
}

fn ticket() -> PairTicketFields {
    PairTicketFields::new(
        PAIR_TICKET_FORMAT_VERSION,
        DeviceId::from_array(HOST_DEVICE),
        "test-host",
        vec![RelayHint::new("https://relay.example.com").expect("valid relay URL")],
        PairOfferId::from_array(OFFER_ID),
        EXPIRES_AT_UNIX,
    )
    .expect("bounded ticket")
}

#[test]
fn ticket_text_round_trips_and_matches_the_canonical_golden_vector() {
    let fields = ticket();
    let secret = PairSecret::from_bytes(SECRET);
    let text = encode_pair_ticket(&fields, &secret);
    assert!(text.starts_with(PAIR_TICKET_PREFIX));
    assert!(text.len() <= zterm_core::MAX_TICKET_TEXT_BYTES);
    // base64url-no-pad never contains padding or base64 non-alphabet.
    assert!(!text[PAIR_TICKET_PREFIX.len()..].contains('='));
    assert!(!text[PAIR_TICKET_PREFIX.len()..].contains('/'));
    assert!(!text[PAIR_TICKET_PREFIX.len()..].contains('+'));

    let (decoded_fields, decoded_secret) = decode_pair_ticket(&text).expect("ticket decodes");
    assert_eq!(decoded_fields, fields);
    assert_eq!(decoded_secret, secret);
    assert_eq!(
        decoded_fields.canonical_bytes().to_vec(),
        hex(TICKET_CANONICAL_HEX)
    );
    assert_eq!(
        decoded_fields.ticket_digest().to_vec(),
        hex(TICKET_DIGEST_HEX)
    );
}

#[test]
fn ticket_text_rejects_prefix_padding_alphabet_truncation_and_oversize() {
    let fields = ticket();
    let secret = PairSecret::from_bytes(SECRET);
    let text = encode_pair_ticket(&fields, &secret);
    let encoded = &text[PAIR_TICKET_PREFIX.len()..];

    // Missing prefix.
    assert!(matches!(
        decode_pair_ticket(encoded),
        Err(zterm_proto::TicketTextError::MissingPrefix)
    ));

    // Padding is not accepted by the no-pad alphabet.
    assert!(matches!(
        decode_pair_ticket(&format!("{PAIR_TICKET_PREFIX}{encoded}=")),
        Err(zterm_proto::TicketTextError::InvalidBase64(_))
    ));

    // Truncated payload fails to decode.
    let truncated = &encoded[..encoded.len() - 3];
    assert!(decode_pair_ticket(&format!("{PAIR_TICKET_PREFIX}{truncated}")).is_err());

    // Oversize text is rejected before base64 decoding.
    let oversized = format!(
        "{PAIR_TICKET_PREFIX}{}",
        "A".repeat(zterm_core::MAX_TICKET_TEXT_BYTES)
    );
    assert!(matches!(
        decode_pair_ticket(&oversized),
        Err(zterm_proto::TicketTextError::TooLong { .. })
    ));
}

#[test]
fn ticket_proto_validates_id_secret_url_count_and_version() {
    let make =
        |offer_id: Vec<u8>, secret: Vec<u8>, relay_urls: Vec<String>, format_version: u32| {
            v1::PairTicketV1 {
                format_version,
                host_device_id: Some(v1::DeviceId {
                    value: HOST_DEVICE.to_vec(),
                }),
                host_name: "test-host".to_owned(),
                relay_urls,
                offer_id,
                secret,
                expires_at_unix: EXPIRES_AT_UNIX,
            }
        };

    // Valid message converts.
    let valid = make(
        OFFER_ID.to_vec(),
        SECRET.to_vec(),
        vec!["https://relay.example.com".to_owned()],
        PAIR_TICKET_FORMAT_VERSION,
    );
    let (fields, secret) = <(PairTicketFields, PairSecret)>::try_from(valid).expect("valid ticket");
    assert_eq!(fields, ticket());
    assert_eq!(secret, PairSecret::from_bytes(SECRET));

    // Wrong offer ID length.
    assert!(matches!(
        <(PairTicketFields, PairSecret)>::try_from(make(
            vec![0; 15],
            SECRET.to_vec(),
            vec!["https://relay.example.com".to_owned()],
            PAIR_TICKET_FORMAT_VERSION,
        )),
        Err(zterm_proto::TicketTextError::InvalidIdentifier(_))
    ));

    // Wrong secret length.
    assert!(matches!(
        <(PairTicketFields, PairSecret)>::try_from(make(
            OFFER_ID.to_vec(),
            vec![0; 31],
            vec!["https://relay.example.com".to_owned()],
            PAIR_TICKET_FORMAT_VERSION,
        )),
        Err(zterm_proto::TicketTextError::InvalidSecret(_))
    ));

    // Non-HTTPS relay URL.
    assert!(matches!(
        <(PairTicketFields, PairSecret)>::try_from(make(
            OFFER_ID.to_vec(),
            SECRET.to_vec(),
            vec!["http://relay.example.com".to_owned()],
            PAIR_TICKET_FORMAT_VERSION,
        )),
        Err(zterm_proto::TicketTextError::InvalidRelayHint(_))
    ));

    // Too many relay hints.
    let many = (0..5)
        .map(|index| format!("https://relay{index}.example.com"))
        .collect();
    assert!(matches!(
        <(PairTicketFields, PairSecret)>::try_from(make(
            OFFER_ID.to_vec(),
            SECRET.to_vec(),
            many,
            PAIR_TICKET_FORMAT_VERSION,
        )),
        Err(zterm_proto::TicketTextError::InvalidTicket(_))
    ));

    // Unsupported format version.
    assert!(matches!(
        <(PairTicketFields, PairSecret)>::try_from(make(
            OFFER_ID.to_vec(),
            SECRET.to_vec(),
            vec!["https://relay.example.com".to_owned()],
            2,
        )),
        Err(zterm_proto::TicketTextError::InvalidTicket(_))
    ));
}

#[test]
fn pair_connection_and_local_device_adapters_round_trip_validated_values() {
    let begin = PairBegin::new(
        PairOfferId::from_array([1; 16]),
        "controller",
        PairNonce::from_array([2; 32]),
        PAIR_PROTOCOL_VERSION,
    )
    .expect("pair begin validates");
    assert_eq!(
        PairBegin::try_from(v1::PairBegin::from(&begin)).expect("pair begin round-trips"),
        begin
    );

    let challenge = PairChallenge::new(
        PairNonce::from_array([3; 32]),
        PAIR_PROTOCOL_VERSION,
        EXPIRES_AT_UNIX,
    )
    .expect("pair challenge validates");
    assert_eq!(
        PairChallenge::try_from(v1::PairChallenge::from(&challenge))
            .expect("pair challenge round-trips"),
        challenge
    );

    let proof = PairProof::from_bytes([4; 32]);
    assert_eq!(
        PairProof::try_from(v1::PairProof::from(&proof)).expect("pair proof round-trips"),
        proof
    );

    let generation = AuthGeneration::new(7).expect("generation validates");
    let accepted =
        PairAccepted::new(generation, [5; 32], "0.1.1").expect("pair acceptance validates");
    assert_eq!(
        PairAccepted::try_from(v1::PairAccepted::from(&accepted))
            .expect("pair acceptance round-trips"),
        accepted
    );

    let hello = ConnectionHello::new(
        1,
        1,
        Capabilities::from_bits_retain(u64::MAX),
        ConnectionAttemptId::from_array([6; 16]),
        "controller",
        "0.1.1",
        "test",
    )
    .expect("hello validates");
    assert_eq!(
        ConnectionHello::try_from(v1::ConnectionHello::from(&hello)).expect("hello round-trips"),
        hello
    );

    let welcome = ConnectionWelcome::new(
        1,
        Capabilities::from_bits_retain(u64::MAX),
        "host",
        "0.1.1",
        "test",
        generation,
    )
    .expect("welcome validates");
    assert_eq!(
        ConnectionWelcome::try_from(v1::ConnectionWelcome::from(&welcome))
            .expect("welcome round-trips"),
        welcome
    );

    let device_id = DeviceId::from_array([7; 32]);
    let alias = DeviceAlias::new("workstation").expect("alias validates");
    let rename = v1::LocalDeviceRenameRequest {
        device_id: Some(device_id.into()),
        alias: alias.as_str().to_owned(),
    };
    assert_eq!(
        <(DeviceId, DeviceAlias)>::try_from(rename).expect("rename request validates"),
        (device_id, alias)
    );
    let revoke = v1::LocalDeviceRevokeRequest {
        device_id: Some(device_id.into()),
    };
    assert_eq!(
        DeviceId::try_from(revoke).expect("revoke request validates"),
        device_id
    );
}

#[test]
fn pairing_handshake_budget_uses_checked_cumulative_accounting() {
    let mut budget = PairHandshakeBudget::new();
    budget.record(16 * 1024).expect("first frame fits");
    budget.record(48 * 1024).expect("exact ceiling fits");
    assert_eq!(budget.used(), 64 * 1024);
    assert_eq!(budget.remaining(), 0);
    assert!(budget.record(1).is_err());

    let mut overflow = PairHandshakeBudget::new();
    assert!(overflow.record(usize::MAX).is_err());
    assert_eq!(overflow.used(), 0);
}

#[test]
fn unknown_fields_and_capability_bits_survive_round_trip() {
    // Capability bits retain unknown bits end to end.
    let hello = v1::ConnectionHello {
        min_wire_major: 1,
        max_wire_major: 1,
        capabilities: u64::MAX,
        attempt_id: vec![7; 16],
        initiator_display: "laptop".to_owned(),
        initiator_build: "0.1.1".to_owned(),
        initiator_platform: "macos".to_owned(),
    };
    let bytes = hello.encode_to_vec();
    let decoded = v1::ConnectionHello::decode(bytes.as_slice()).expect("hello decodes");
    assert_eq!(decoded, hello);
    assert_eq!(decoded.capabilities, u64::MAX);
    let capabilities = Capabilities::from_bits_retain(decoded.capabilities);
    assert_eq!(capabilities.bits(), u64::MAX);
    assert!(ConnectionAttemptId::from_bytes(&decoded.attempt_id).is_ok());

    // An unknown trailing protobuf field is ignored by a compatible decoder.
    let mut with_unknown = bytes;
    with_unknown.extend_from_slice(&[0xf8, 0x07, 0x01]); // field 255, varint 1
    let decoded = v1::ConnectionHello::decode(with_unknown.as_slice()).expect("unknown field kept");
    assert_eq!(decoded, hello);
}

#[test]
fn relay_route_cache_round_trips_and_rejects_unknown_versions() {
    let urls = [
        RelayHint::new("https://relay.example.com").expect("valid relay URL"),
        RelayHint::new("https://second.example.com").expect("valid relay URL"),
    ];
    let bytes = encode_relay_route_cache(&urls).expect("bounded cache encodes");
    assert_eq!(
        decode_relay_route_cache(&bytes).expect("route cache decodes"),
        urls
    );

    // An unknown cache version fails with a diagnostic and is never migrated.
    let unknown = v1::RelayRouteCacheV1 {
        format_version: 2,
        relay_urls: vec!["https://relay.example.com".to_owned()],
    };
    assert!(matches!(
        decode_relay_route_cache(&unknown.encode_to_vec()),
        Err(zterm_proto::RouteCacheError::UnsupportedVersion { actual: 2 })
    ));
    assert_eq!(RELAY_ROUTE_CACHE_VERSION, 1);
}

#[test]
fn relay_route_cache_enforces_count_and_byte_ceiling() {
    assert!(matches!(
        encode_relay_route_cache(&[]),
        Err(zterm_proto::RouteCacheError::MissingUrl)
    ));
    let empty = v1::RelayRouteCacheV1 {
        format_version: RELAY_ROUTE_CACHE_VERSION,
        relay_urls: Vec::new(),
    };
    assert!(matches!(
        decode_relay_route_cache(&empty.encode_to_vec()),
        Err(zterm_proto::RouteCacheError::MissingUrl)
    ));

    // Encoding an oversized slice is rejected outright.
    let too_many: Vec<RelayHint> = (0..5)
        .map(|index| {
            RelayHint::new(format!("https://relay{index}.example.com")).expect("valid URL")
        })
        .collect();
    assert!(matches!(
        encode_relay_route_cache(&too_many),
        Err(zterm_proto::RouteCacheError::TooManyUrls { actual: 5 })
    ));

    // A decoded cache advertising more than the bound is rejected.
    let oversized = v1::RelayRouteCacheV1 {
        format_version: RELAY_ROUTE_CACHE_VERSION,
        relay_urls: (0..5)
            .map(|index| format!("https://relay{index}.example.com"))
            .collect(),
    };
    assert!(matches!(
        decode_relay_route_cache(&oversized.encode_to_vec()),
        Err(zterm_proto::RouteCacheError::TooManyUrls { actual: 5 })
    ));

    // A pre-decode blob over the byte ceiling is rejected before allocation.
    let huge = vec![0_u8; zterm_proto::MAX_RELAY_ROUTE_CACHE_BYTES + 1];
    assert!(matches!(
        decode_relay_route_cache(&huge),
        Err(zterm_proto::RouteCacheError::TooLarge { .. })
    ));
}

#[test]
fn wire_kind_registry_is_unique_and_centrally_mapped() {
    use std::collections::BTreeSet;

    let kinds = [
        WireKind::LocalReadinessRequest,
        WireKind::LocalReadinessResponse,
        WireKind::LocalStatusRequest,
        WireKind::LocalStatusResponse,
        WireKind::LocalValidateSetupRequest,
        WireKind::LocalValidateSetupResponse,
        WireKind::LocalStopRequest,
        WireKind::LocalStopResponse,
        WireKind::LocalUpdatePreflightRequest,
        WireKind::LocalUpdatePreflightResponse,
        WireKind::ServiceErrorResponse,
        WireKind::LocalPairCreateRequest,
        WireKind::LocalPairCreateResponse,
        WireKind::LocalPairAcceptRequest,
        WireKind::LocalPairAcceptResponse,
        WireKind::LocalDeviceListRequest,
        WireKind::LocalDeviceListResponse,
        WireKind::LocalDeviceRenameRequest,
        WireKind::LocalDeviceRenameResponse,
        WireKind::LocalDeviceRevokeRequest,
        WireKind::LocalDeviceRevokeResponse,
        WireKind::LocalTargetResolveRequest,
        WireKind::LocalTargetResolveResponse,
        WireKind::LocalSessionUnaryRequest,
        WireKind::PairBegin,
        WireKind::PairChallenge,
        WireKind::PairProof,
        WireKind::PairAccepted,
        WireKind::ConnectionHello,
        WireKind::ConnectionWelcome,
        WireKind::SessionListRequest,
        WireKind::SessionListResponse,
        WireKind::SessionCreateRequest,
        WireKind::SessionMutateResponse,
        WireKind::SessionRenameRequest,
        WireKind::SessionCloseRequest,
        WireKind::SessionTakeoverRequest,
        WireKind::SessionOperationLeaseRequest,
        WireKind::SessionOperationLeaseResponse,
        WireKind::TerminalAttachRequest,
        WireKind::TerminalSnapshot,
        WireKind::TerminalDelta,
        WireKind::TerminalInput,
        WireKind::TerminalResize,
        WireKind::TerminalDetach,
        WireKind::TerminalSnapshotApplied,
        WireKind::TerminalSyncRequest,
        WireKind::TerminalSyncRequired,
        WireKind::TerminalLeaseLost,
        WireKind::TerminalSessionEnded,
        WireKind::TerminalTransportStateEvent,
        WireKind::TerminalHistoryRequest,
        WireKind::TerminalHistoryPage,
        WireKind::TerminalConnectionStatusEvent,
        WireKind::TerminalViewportRequest,
        WireKind::TerminalViewportFrame,
        WireKind::TerminalHistoryWindowRequest,
        WireKind::TerminalHistoryWindowFrame,
    ];

    let mut seen = BTreeSet::new();
    for kind in kinds {
        let number = kind as u32;
        assert!(seen.insert(number), "duplicate kind number {number}");
        assert_eq!(WireKind::try_from(number).expect("mapped kind"), kind);
    }

    // Local pair/device kinds occupy 12..=21, target/session forwarding uses
    // 22..=24, and pair/transport remains 100..=105.
    assert_eq!(WireKind::LocalPairCreateRequest as u32, 12);
    assert_eq!(WireKind::LocalDeviceRevokeResponse as u32, 21);
    assert_eq!(WireKind::LocalTargetResolveRequest as u32, 22);
    assert_eq!(WireKind::LocalSessionUnaryRequest as u32, 24);
    assert_eq!(WireKind::PairBegin as u32, 100);
    assert_eq!(WireKind::ConnectionWelcome as u32, 105);
    assert_eq!(WireKind::TerminalTransportStateEvent as u32, 311);
    assert_eq!(WireKind::TerminalHistoryRequest as u32, 312);
    assert_eq!(WireKind::TerminalHistoryPage as u32, 313);
    assert_eq!(WireKind::TerminalConnectionStatusEvent as u32, 314);
    assert_eq!(WireKind::TerminalViewportRequest as u32, 315);
    assert_eq!(WireKind::TerminalViewportFrame as u32, 316);
    assert_eq!(WireKind::TerminalHistoryWindowRequest as u32, 317);
    assert_eq!(WireKind::TerminalHistoryWindowFrame as u32, 318);
    assert_eq!(Capabilities::HISTORY_PAGING, 1_u64 << 17);
    assert_eq!(Capabilities::AGENT_EVENTS, 1_u64 << 18);
    assert_eq!(Capabilities::TERMINAL_VIEWPORT, 1_u64 << 19);
    assert_eq!(Capabilities::TERMINAL_HISTORY_WINDOW, 1_u64 << 20);
}

#[test]
fn device_summary_exposes_each_direction_explicitly() {
    let device = v1::DeviceSummary {
        device_id: Some(v1::DeviceId { value: vec![7; 32] }),
        outbound_known: true,
        alias: String::new(),
        remote_name: "laptop".to_owned(),
        route_verified: true,
        auth_status: v1::DeviceAuthStatus::None as i32,
        generation: 0,
        paired_at_unix: 0,
        last_seen_at_unix: 1_700_000_000,
        online: false,
        active_stream_count: 0,
        remote_attachment_count: 0,
    };
    let bytes = device.encode_to_vec();
    let decoded = v1::DeviceSummary::decode(bytes.as_slice()).expect("device summary decodes");
    assert_eq!(decoded, device);
    // An empty alias must not be the way a consumer learns the outbound row is
    // absent: `outbound_known` is the explicit direction.
    assert!(decoded.outbound_known);
    assert!(decoded.alias.is_empty());
    assert_eq!(decoded.auth_status, v1::DeviceAuthStatus::None as i32);
    let domain = DeviceSummary::try_from(decoded).expect("directional summary validates");
    assert!(domain.outbound_known());

    for invalid in [v1::DeviceAuthStatus::Unspecified as i32, 99] {
        let mut malformed = device.clone();
        malformed.auth_status = invalid;
        assert!(matches!(
            DeviceSummary::try_from(malformed),
            Err(WireFieldError::InvalidAuthStatus { actual }) if actual == invalid
        ));
    }
}

#[test]
fn handshake_adapters_reject_zero_protocol_and_authorization_sentinels() {
    let zero_range = v1::ConnectionHello {
        min_wire_major: 0,
        max_wire_major: 0,
        capabilities: 0,
        attempt_id: vec![1; 16],
        initiator_display: "peer".to_owned(),
        initiator_build: "0.1.1".to_owned(),
        initiator_platform: "test".to_owned(),
    };
    assert!(matches!(
        ConnectionHello::try_from(zero_range),
        Err(WireFieldError::InvalidConnection(_))
    ));

    let zero_welcome = v1::ConnectionWelcome {
        wire_major: 0,
        capabilities: 0,
        responder_display: "host".to_owned(),
        responder_build: "0.1.1".to_owned(),
        responder_platform: "test".to_owned(),
        accepted_authorization_generation: 1,
    };
    assert!(matches!(
        ConnectionWelcome::try_from(zero_welcome),
        Err(WireFieldError::InvalidConnection(_))
    ));

    let zero_generation = v1::PairAccepted {
        authorization_generation: 0,
        host_confirmation_proof: vec![2; 32],
        host_diagnostic_version: "0.1.1".to_owned(),
    };
    assert!(matches!(
        PairAccepted::try_from(zero_generation),
        Err(WireFieldError::InvalidPair(_))
    ));
}

#[test]
fn generated_pair_messages_and_decoded_frames_redact_sensitive_payloads() {
    const FRAME_SENTINEL: &[u8] = b"PAIR_FRAME_SENTINEL_c5e7";
    const OFFER_SENTINEL: &[u8; 16] = b"OFFER_SENTINEL_1";
    const NONCE_SENTINEL: &[u8; 32] = b"PAIR_NONCE_SENTINEL_0123456789AB";
    const PROOF_SENTINEL: &[u8; 32] = b"PAIR_PROOF_SENTINEL_0123456789AB";
    const KEY_SENTINEL: &[u8; 32] = b"PAIR_KEY_SENTINEL_0123456789ABCD";
    const TICKET_SENTINEL: &str = "zterm-pair-v1:PAIR_TICKET_SENTINEL_470b";

    let wire = v1::WireFrame {
        wire_major: WIRE_MAJOR,
        kind: WireKind::PairProof as u32,
        payload: FRAME_SENTINEL.to_vec(),
        request_id: 7,
        deadline_ms: 8,
    };
    let decoded = DecodedFrame {
        kind: WireKind::PairProof,
        request_id: 7,
        deadline_ms: 8,
        payload: FRAME_SENTINEL.to_vec(),
    };
    let ticket = v1::PairTicketV1 {
        format_version: PAIR_TICKET_FORMAT_VERSION,
        host_device_id: None,
        host_name: "host".to_owned(),
        relay_urls: vec!["https://relay.example.test".to_owned()],
        offer_id: OFFER_SENTINEL.to_vec(),
        secret: KEY_SENTINEL.to_vec(),
        expires_at_unix: EXPIRES_AT_UNIX,
    };
    let begin = v1::PairBegin {
        offer_id: OFFER_SENTINEL.to_vec(),
        controller_name: "controller".to_owned(),
        controller_nonce: NONCE_SENTINEL.to_vec(),
        pair_protocol_version: PAIR_PROTOCOL_VERSION,
    };
    let challenge = v1::PairChallenge {
        host_nonce: NONCE_SENTINEL.to_vec(),
        selected_version: PAIR_PROTOCOL_VERSION,
        ticket_expiry_unix: EXPIRES_AT_UNIX,
    };
    let proof = v1::PairProof {
        controller_proof: PROOF_SENTINEL.to_vec(),
    };
    let accepted = v1::PairAccepted {
        authorization_generation: 1,
        host_confirmation_proof: PROOF_SENTINEL.to_vec(),
        host_diagnostic_version: "test-build".to_owned(),
    };
    let local_create = v1::LocalPairCreateResponse {
        ticket: TICKET_SENTINEL.to_owned(),
    };
    let local_accept = v1::LocalPairAcceptRequest {
        ephemeral_operation_id: OFFER_SENTINEL.to_vec(),
        fingerprint: KEY_SENTINEL.to_vec(),
        ticket: TICKET_SENTINEL.to_owned(),
        alias: "host".to_owned(),
    };
    let rendered = format!(
        "{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
        wire, decoded, ticket, begin, challenge, proof, accepted, local_create, local_accept,
    );
    assert!(rendered.contains("[REDACTED]"));
    for bytes in [
        FRAME_SENTINEL,
        OFFER_SENTINEL,
        NONCE_SENTINEL,
        PROOF_SENTINEL,
        KEY_SENTINEL,
    ] {
        assert!(!rendered.contains(std::str::from_utf8(bytes).expect("ASCII sentinel")));
        assert!(!rendered.contains(&format!("{bytes:?}")));
    }
    assert!(!rendered.contains(TICKET_SENTINEL));

    assert_message_round_trip(&wire);
    assert_message_round_trip(&ticket);
    assert_message_round_trip(&begin);
    assert_message_round_trip(&challenge);
    assert_message_round_trip(&proof);
    assert_message_round_trip(&accepted);
    assert_message_round_trip(&local_create);
    assert_message_round_trip(&local_accept);
}

#[test]
fn generated_session_terminal_and_route_debug_is_redacted_without_wire_changes() {
    const CWD_SENTINEL: &str = "/private/tmp/CWD_SENTINEL_8b62/project";
    const RELAY_SENTINEL: &str = "https://RELAY_ROUTE_SENTINEL_60e4.example.test/path";
    const HOME_RELAY_SENTINEL: &str = "https://HOME_RELAY_SENTINEL_19a5.example.test";
    const SCREEN_SENTINEL: &[u8] = b"PROTO_SCREEN_SENTINEL_e357";
    const HISTORY_SENTINEL: &[u8] = b"PROTO_HISTORY_SENTINEL_4d18";
    const DELTA_SENTINEL: &[u8] = b"PROTO_DELTA_SENTINEL_729a";
    const INPUT_SENTINEL: &[u8] = b"PROTO_INPUT_SENTINEL_b84f";
    const RESUME_SENTINEL: &[u8; 16] = b"RESUME_PROTO_4d2";

    let summary = v1::SessionSummary {
        session_id: Some(v1::SessionId {
            value: vec![0x31; 16],
        }),
        name: "build".to_owned(),
        revision: 37,
        has_controller: true,
        working_directory: CWD_SENTINEL.to_owned(),
        viewport: Some(v1::TerminalViewport {
            rows: 43,
            columns: 151,
        }),
    };
    let create = v1::SessionCreateRequest {
        operation_id: None,
        target: None,
        name: "build".to_owned(),
        working_directory: CWD_SENTINEL.to_owned(),
        viewport: summary.viewport,
    };
    let snapshot = v1::TerminalSnapshot {
        session_id: summary.session_id.clone(),
        attachment_id: Some(v1::AttachmentId {
            value: vec![0x32; 16],
        }),
        revision: 41,
        rows: 43,
        columns: 151,
        screen_ansi: SCREEN_SENTINEL.to_vec(),
        recent_history_ansi: HISTORY_SENTINEL.to_vec(),
        active_screen: v1::TerminalActiveScreen::Main as i32,
        modes: Some(v1::TerminalModes::default()),
        scroll_metrics: None,
    };
    let delta = v1::TerminalDelta {
        from_revision: 41,
        to_revision: 47,
        ansi: DELTA_SENTINEL.to_vec(),
        rows: 43,
        columns: 151,
        active_screen: v1::TerminalActiveScreen::Main as i32,
        modes: Some(v1::TerminalModes::default()),
        attachment_id: snapshot.attachment_id.clone(),
        scroll_metrics: None,
    };
    let input = v1::TerminalInput {
        operation_id: None,
        attachment_id: snapshot.attachment_id.clone(),
        bytes: INPUT_SENTINEL.to_vec(),
    };
    let resume_view_id = v1::ResumeViewId {
        value: RESUME_SENTINEL.to_vec(),
    };
    let attach = v1::TerminalAttachRequest {
        target: None,
        session_id: summary.session_id.clone(),
        takeover: false,
        session_name: "build".to_owned(),
        create_main: false,
        viewport: summary.viewport,
        resume_view_id: Some(resume_view_id.clone()),
        known_revision: Some(37),
    };
    let route_cache = v1::RelayRouteCacheV1 {
        format_version: RELAY_ROUTE_CACHE_VERSION,
        relay_urls: vec![RELAY_SENTINEL.to_owned()],
    };
    let status = v1::LocalStatusResponse {
        home_relay: HOME_RELAY_SENTINEL.to_owned(),
        active_session_count: 1,
        active_session_names: vec!["build".to_owned()],
        direct_path_count: 2,
        relay_path_count: 3,
        ..v1::LocalStatusResponse::default()
    };
    let validate_setup = v1::LocalValidateSetupRequest {
        device_name: "host".to_owned(),
        infrastructure_profile: "self-hosted".to_owned(),
        relay_url: RELAY_SENTINEL.to_owned(),
    };
    let list = v1::SessionListResponse {
        sessions: vec![summary.clone()],
    };
    let mutate = v1::SessionMutateResponse {
        session: Some(summary.clone()),
    };

    let rendered = format!(
        "{summary:?} {create:?} {resume_view_id:?} {attach:?} {snapshot:?} {delta:?} \
         {input:?} {route_cache:?} {status:?} {validate_setup:?} {list:?} {mutate:?}"
    );
    for text in [CWD_SENTINEL, RELAY_SENTINEL, HOME_RELAY_SENTINEL] {
        assert!(!rendered.contains(text));
    }
    for bytes in [
        SCREEN_SENTINEL,
        HISTORY_SENTINEL,
        DELTA_SENTINEL,
        INPUT_SENTINEL,
        RESUME_SENTINEL,
    ] {
        assert!(!rendered.contains(std::str::from_utf8(bytes).expect("ASCII sentinel")));
        assert!(!rendered.contains(&format!("{bytes:?}")));
    }
    assert!(rendered.contains("[REDACTED]"));
    assert!(rendered.contains("revision: 37"));
    assert!(rendered.contains("rows: 43"));
    assert!(rendered.contains("columns: 151"));
    assert!(rendered.contains("relay_url_count: 1"));
    assert!(rendered.contains(&format!("input_len: {}", INPUT_SENTINEL.len())));
    assert!(rendered.contains("direct_path_count: 2"));
    assert!(rendered.contains("relay_path_count: 3"));

    assert_message_round_trip(&summary);
    assert_message_round_trip(&create);
    assert_message_round_trip(&resume_view_id);
    assert_message_round_trip(&attach);
    assert_message_round_trip(&snapshot);
    assert_message_round_trip(&delta);
    assert_message_round_trip(&input);
    assert_message_round_trip(&route_cache);
    assert_message_round_trip(&status);
    assert_message_round_trip(&validate_setup);
}

#[test]
fn pair_operation_identity_is_exact_and_bounded_before_allocation() {
    let operation_id = [0x55; 16];
    let fingerprint = [0x77; 32];

    let (id, print) = validate_pair_operation(&operation_id, &fingerprint)
        .expect("exact id and bounded fingerprint validate");
    assert_eq!(id, EphemeralOperationId::from_array([0x55; 16]));
    assert_eq!(print.as_bytes(), &fingerprint);

    // A digest-sized fingerprint is accepted and its Debug never reveals bytes.
    let digest = [0x11; PAIR_FINGERPRINT_BYTES];
    let (_, print) = validate_pair_operation(&operation_id, &digest)
        .expect("digest-width fingerprint validates");
    assert_eq!(print.as_bytes(), &digest);
    assert_eq!(PairFingerprint::from_bytes(digest).as_bytes(), &digest);
    // Debug reveals no digest bytes, even for attacker-selected input.
    let rendered = format!("{:?}", PairFingerprint::from_bytes([0xab; 32]));
    assert_eq!(rendered, "PairFingerprint([REDACTED])");
    assert!(!rendered.contains("ab"));

    // Wrong ephemeral operation ID length is rejected before allocation.
    assert!(matches!(
        validate_pair_operation(&[0; 15], &fingerprint),
        Err(zterm_proto::PairOperationError::InvalidOperationId(_))
    ));

    // An under-width fingerprint is rejected.
    assert!(matches!(
        validate_pair_operation(&operation_id, &[]),
        Err(zterm_proto::PairOperationError::InvalidFingerprint(
            PairFingerprintError::InvalidLength { actual: 0 }
        ))
    ));

    // An over-width fingerprint is rejected before allocation.
    assert!(matches!(
        validate_pair_operation(&operation_id, &[0; PAIR_FINGERPRINT_BYTES + 1]),
        Err(zterm_proto::PairOperationError::InvalidFingerprint(
            PairFingerprintError::InvalidLength { .. }
        ))
    ));
}

#[test]
fn ticket_decode_failure_paths_never_leak_the_secret_sentinel() {
    const SENTINEL: u8 = 0x5e; // '^'
    let secret = vec![SENTINEL; 32];

    // Invalid host identity returns before the secret is otherwise used.
    let bad_host = v1::PairTicketV1 {
        host_device_id: Some(v1::DeviceId { value: vec![0; 31] }),
        secret: secret.clone(),
        ..good_ticket()
    };
    let Err(err) = <(PairTicketFields, PairSecret)>::try_from(bad_host) else {
        panic!("bad host identity must fail validation");
    };
    assert!(
        !format!("{err}").contains("5e"),
        "secret sentinel must not leak on the failure path: {err}"
    );

    // Invalid offer ID returns before the secret is otherwise used.
    let bad_offer = v1::PairTicketV1 {
        offer_id: vec![0; 15],
        secret,
        ..good_ticket()
    };
    let Err(err) = <(PairTicketFields, PairSecret)>::try_from(bad_offer) else {
        panic!("bad offer id must fail validation");
    };
    assert!(!format!("{err}").contains("5e"));
}

fn good_ticket() -> v1::PairTicketV1 {
    v1::PairTicketV1 {
        format_version: PAIR_TICKET_FORMAT_VERSION,
        host_device_id: Some(v1::DeviceId {
            value: HOST_DEVICE.to_vec(),
        }),
        host_name: "test-host".to_owned(),
        relay_urls: vec!["https://relay.example.com".to_owned()],
        offer_id: OFFER_ID.to_vec(),
        secret: vec![0x5e; 32],
        expires_at_unix: EXPIRES_AT_UNIX,
    }
}

#[test]
fn pair_hello_kinds_use_the_16_kib_codec_limit() {
    // A pair/hello frame at exactly the bound is admitted.
    encode_payload(
        WireKind::PairBegin,
        1,
        0,
        vec![0; MAX_PAIR_HELLO_FRAME_BYTES],
    )
    .expect("exact pair/hello ceiling is accepted");

    // One byte over the 16 KiB bound is rejected before decoding.
    assert!(matches!(
        encode_payload(
            WireKind::PairBegin,
            1,
            0,
            vec![0; MAX_PAIR_HELLO_FRAME_BYTES + 1]
        ),
        Err(zterm_proto::ProtocolError::ControlPayloadTooLarge(_))
    ));
    assert!(matches!(
        encode_payload(
            WireKind::ConnectionHello,
            1,
            0,
            vec![0; MAX_PAIR_HELLO_FRAME_BYTES + 1]
        ),
        Err(zterm_proto::ProtocolError::ControlPayloadTooLarge(_))
    ));

    // The 64 KiB handshake budget is exposed for the daemon consumer.
    assert_eq!(zterm_proto::MAX_PAIR_HANDSHAKE_BYTES, 64 * 1024);
}
