//! Same-UID local IPC handshake, isolation, and graceful-stop acceptance tests.

#[cfg(unix)]
#[path = "support/state_fixture.rs"]
mod state_fixture;

#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use prost::Message;
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use zterm_core::terminal::TerminalSize;
#[cfg(unix)]
use zterm_core::{DeviceAlias, DeviceId, DomainErrorKind, SessionName};
#[cfg(unix)]
use zterm_daemon::bootstrap::bootstrap;
#[cfg(unix)]
use zterm_daemon::config::{ValidatedInfrastructure, validate_setup_input};
#[cfg(unix)]
use zterm_daemon::lifecycle::run_local_only_daemon_for_test;
#[cfg(unix)]
use zterm_daemon::local_ipc::{LocalClient, LocalIpcLimits, serve_local_with_limits};
#[cfg(unix)]
use zterm_daemon::service::DaemonService;
#[cfg(unix)]
use zterm_daemon::store::StateStore;
#[cfg(unix)]
use zterm_platform::local_unix::{DaemonLock, bind_daemon_socket, remove_own_socket};
#[cfg(unix)]
use zterm_proto::{FrameDecoder, WIRE_MAJOR, WireKind, encode_message, v1};

#[cfg(unix)]
use state_fixture::TestState;

#[cfg(unix)]
#[tokio::test]
async fn same_uid_service_errors_are_connection_local_and_stop_ack_is_flushed() {
    let state = TestState::new();
    let requested =
        validate_setup_input("ipc-host", ValidatedInfrastructure::OfficialN0).expect("valid setup");
    let setup = bootstrap(&state.paths, &requested).expect("bootstrap");
    let lock = DaemonLock::try_acquire(&state.paths)
        .expect("daemon lock probe")
        .expect("daemon lock");
    let listener = bind_daemon_socket(&state.paths, &lock).expect("listener");
    let service = Arc::new(DaemonService::with_started_at(setup.clone(), 123));
    let server = tokio::spawn(serve_local_with_limits(
        listener,
        state.paths.uid(),
        service,
        LocalIpcLimits::for_test(Duration::from_millis(200)),
    ));
    let client = LocalClient::new(state.paths.socket());

    let readiness = client.readiness().await.expect("readiness");
    assert_eq!(readiness.started_at_unix, 123);
    assert_eq!(readiness.protocol.wire_major, WIRE_MAJOR);
    let status = client.status().await.expect("status");
    assert_eq!(status.device_id, setup.device_id);
    assert_eq!(status.active_session_count, 0);
    assert!(status.active_session_names.is_empty());
    assert_eq!(
        client
            .validate_setup(&requested)
            .await
            .expect("setup validates")
            .device_id,
        setup.device_id
    );
    let conflicting = validate_setup_input("other", ValidatedInfrastructure::OfficialN0)
        .expect("valid conflicting input");
    assert_eq!(
        client
            .validate_setup(&conflicting)
            .await
            .expect_err("conflict returned")
            .kind(),
        DomainErrorKind::AlreadyConfiguredConflict
    );
    let mixed_profile = encode_message(
        WireKind::LocalValidateSetupRequest,
        10,
        0,
        &v1::LocalValidateSetupRequest {
            device_name: requested.device_name.clone(),
            infrastructure_profile: "official-n0".to_owned(),
            relay_url: "https://relay.example.com".to_owned(),
        },
    )
    .expect("mixed profile request");
    assert_error_code(&state, mixed_profile, "config_profile").await;
    client
        .readiness()
        .await
        .expect("server survives invalid profile");
    assert!(
        !client
            .update_preflight()
            .await
            .expect("preflight")
            .interruption_required
    );

    let mut wrong_major = encode_message(
        WireKind::LocalStatusRequest,
        11,
        0,
        &v1::LocalStatusRequest {},
    )
    .expect("request frame");
    let major = wrong_major
        .windows(2)
        .position(|window| window == [0x08, 0x01])
        .expect("wire-major field")
        + 1;
    wrong_major[major] = 2;
    assert_error_code(&state, wrong_major, "wire_major_mismatch").await;
    client.readiness().await.expect("server survives major");

    let unknown_kind = raw_wire(v1::WireFrame {
        wire_major: WIRE_MAJOR,
        kind: 65_535,
        payload: Vec::new(),
        request_id: 12,
        deadline_ms: 0,
    });
    assert_error_code(&state, unknown_kind, "unknown_kind").await;
    client.readiness().await.expect("server survives kind");

    let mut extra_request = encode_message(
        WireKind::LocalStatusRequest,
        12,
        0,
        &v1::LocalStatusRequest {},
    )
    .expect("status request");
    extra_request.extend_from_slice(&[5, 1]);
    assert_error_code(&state, extra_request, "malformed_frame").await;
    client
        .readiness()
        .await
        .expect("server survives trailing request bytes");

    let mut split_trailing = tokio::net::UnixStream::connect(state.paths.socket())
        .await
        .expect("split-trailing client");
    let split_request = encode_message(
        WireKind::LocalStatusRequest,
        12,
        0,
        &v1::LocalStatusRequest {},
    )
    .expect("split status request");
    split_trailing
        .write_all(&split_request)
        .await
        .expect("complete first frame");
    tokio::time::sleep(Duration::from_millis(20)).await;
    split_trailing
        .write_all(&[5, 1])
        .await
        .expect("later trailing bytes");
    split_trailing.shutdown().await.expect("finish raw request");
    let split_response = read_response(&mut split_trailing).await;
    assert_service_error(&split_response, "malformed_frame");
    client
        .readiness()
        .await
        .expect("server survives split trailing request bytes");

    let mut stalled_unary = tokio::net::UnixStream::connect(state.paths.socket())
        .await
        .expect("stalled-unary client");
    let stalled_request = encode_message(
        WireKind::LocalStatusRequest,
        14,
        40,
        &v1::LocalStatusRequest {},
    )
    .expect("stalled status request");
    stalled_unary
        .write_all(&stalled_request)
        .await
        .expect("complete request without half-close");
    let stalled_response = read_response(&mut stalled_unary).await;
    assert_service_error(&stalled_response, "deadline_exceeded");
    client
        .readiness()
        .await
        .expect("server survives unary half-close deadline");

    let future = encode_message(WireKind::PairBegin, 13, 0, &v1::PairBegin::default())
        .expect("future request");
    assert_error_code(&state, future, "service_not_implemented").await;
    client
        .readiness()
        .await
        .expect("server survives future service");

    assert_error_code(&state, vec![0x81, 0x80, 0x80, 0x04], "frame_too_large").await;
    client.readiness().await.expect("server survives oversize");

    let mut partial = tokio::net::UnixStream::connect(state.paths.socket())
        .await
        .expect("partial client");
    partial.write_all(&[5, 1]).await.expect("partial frame");
    let response = read_response(&mut partial).await;
    assert_service_error(&response, "deadline_exceeded");
    client.readiness().await.expect("server survives deadline");

    let cancelled = tokio::net::UnixStream::connect(state.paths.socket())
        .await
        .expect("cancelled client");
    drop(cancelled);
    client
        .readiness()
        .await
        .expect("server survives client close");

    let stop = client.stop(false).await.expect("stop response");
    assert!(stop.stopping);
    assert_eq!(stop.active_session_count, 0);
    server.await.expect("server task").expect("server result");
    remove_own_socket(&state.paths, &lock).expect("socket cleanup");
    assert!(!state.paths.socket().exists());
}

#[cfg(unix)]
#[tokio::test]
async fn local_only_daemon_resolves_exact_targets_but_never_touches_network() {
    let state = TestState::new();
    let requested = validate_setup_input(
        "local-only-remote-session",
        ValidatedInfrastructure::OfficialN0,
    )
    .expect("valid setup");
    let setup = bootstrap(&state.paths, &requested).expect("bootstrap");
    let target = DeviceId::from_array([0xd1; 32]);
    let inbound_only = DeviceId::from_array([0xd2; 32]);
    let short_hex_alias = DeviceId::from_array([0xd3; 32]);
    let mut store = StateStore::open(&state.paths).expect("open committed state");
    store
        .upsert_known_device(
            target,
            &DeviceAlias::new("offline-host").expect("alias"),
            "Offline host",
            None,
        )
        .expect("known outbound target");
    store
        .upsert_known_device(
            short_hex_alias,
            &DeviceAlias::new("abc").expect("short hex-only alias"),
            "Short alias host",
            None,
        )
        .expect("known short-alias target");
    store
        .upsert_known_device(
            setup.device_id,
            &DeviceAlias::new("self-host").expect("self alias"),
            "This device",
            None,
        )
        .expect("self-shaped outbound fixture");
    store
        .authorize_device(inbound_only, "Inbound controller", 1)
        .expect("inbound-only authorization");
    drop(store);

    let paths = state.paths.clone();
    let daemon = std::thread::spawn(move || run_local_only_daemon_for_test(&paths));
    let client = LocalClient::new(state.paths.socket());
    let readiness_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match client.readiness().await {
            Ok(_) => break,
            Err(error)
                if error.kind() == DomainErrorKind::DaemonStopped
                    && tokio::time::Instant::now() < readiness_deadline =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => panic!("local-only daemon did not become ready: {error}"),
        }
    }

    let local = client
        .resolve_session_target("local")
        .await
        .expect("reserved local target");
    assert!(local.is_local());
    assert!(
        client
            .list_sessions_at(local)
            .await
            .expect("local Session wire path remains available")
            .is_empty()
    );
    let remote = client
        .resolve_session_target("offline-host")
        .await
        .expect("daemon-owned exact alias resolution");
    assert_eq!(remote.device_id(), Some(target));
    assert_eq!(
        client
            .resolve_session_target("abc")
            .await
            .expect("an exact short hex-only alias is not parsed as an ID prefix")
            .device_id(),
        Some(short_hex_alias)
    );
    for self_selector in ["self-host".to_owned(), setup.device_id.to_string()] {
        assert_eq!(
            client
                .resolve_session_target(&self_selector)
                .await
                .expect_err("the local identity must use the reserved local target")
                .kind(),
            DomainErrorKind::InvalidTargetSelector
        );
    }
    assert_eq!(
        client
            .resolve_session_target(&target.to_string().to_uppercase())
            .await
            .expect_err("uppercase ID is invalid across local IPC")
            .kind(),
        DomainErrorKind::InvalidTargetSelector
    );
    assert_eq!(
        client
            .resolve_session_target(&target.to_string()[..8])
            .await
            .expect_err("prefix ID is invalid across local IPC")
            .kind(),
        DomainErrorKind::InvalidTargetSelector
    );
    assert_eq!(
        client
            .resolve_session_target(&inbound_only.to_string())
            .await
            .expect_err("inbound-only target remains directionally denied")
            .kind(),
        DomainErrorKind::OutboundDirectionDenied
    );
    assert_eq!(
        client
            .resolve_session_target("unknown-host")
            .await
            .expect_err("unknown exact alias remains not found")
            .kind(),
        DomainErrorKind::DeviceNotFound
    );
    assert_eq!(
        client
            .list_sessions_at(remote)
            .await
            .expect_err("local-only daemon has no remote transport owner")
            .kind(),
        DomainErrorKind::TransportUnavailable
    );
    assert!(
        client
            .list_sessions()
            .await
            .expect("remote failure does not poison local Session service")
            .is_empty()
    );

    client.stop(false).await.expect("stop local-only daemon");
    tokio::task::spawn_blocking(move || daemon.join().expect("daemon thread joins"))
        .await
        .expect("daemon join worker")
        .expect("local-only daemon result");
}

#[cfg(unix)]
#[tokio::test]
async fn ambiguous_mutation_retry_reuses_byte_identical_request_and_operation_id() {
    let temporary = tempfile::tempdir().expect("temporary retry fixture");
    let socket = temporary.path().join("retry.sock");
    let listener = tokio::net::UnixListener::bind(&socket).expect("bind retry fixture");
    let server = tokio::spawn(async move {
        let (mut lease_stream, _) = listener.accept().await.expect("accept lease request");
        let lease_bytes = read_raw_request(&mut lease_stream).await;
        let lease_frame = decode_raw_request(&lease_bytes);
        assert_eq!(lease_frame.kind, WireKind::SessionOperationLeaseRequest);
        let lease_response = encode_message(
            WireKind::SessionOperationLeaseResponse,
            lease_frame.request_id,
            0,
            &v1::SessionOperationLeaseResponse {
                lease: Some(v1::OperationLease {
                    daemon_incarnation: vec![7; 16],
                    ordinal: 1,
                }),
            },
        )
        .expect("encode lease response");
        lease_stream
            .write_all(&lease_response)
            .await
            .expect("write lease response");
        lease_stream
            .shutdown()
            .await
            .expect("finish lease response");

        let (mut abandoned, _) = listener.accept().await.expect("accept first mutation");
        let first_bytes = read_raw_request(&mut abandoned).await;
        let first_frame = decode_raw_request(&first_bytes);
        assert_eq!(first_frame.kind, WireKind::SessionCreateRequest);
        let first: v1::SessionCreateRequest = first_frame
            .decode_message(WireKind::SessionCreateRequest)
            .expect("decode first mutation");
        let operation_key = first
            .operation_id
            .as_ref()
            .expect("first mutation operation ID")
            .encode_to_vec();
        let mut seen = std::collections::BTreeSet::new();
        let mut executions = usize::from(seen.insert(operation_key));
        drop(abandoned);

        let (mut retried, _) = listener.accept().await.expect("accept retried mutation");
        let retry_bytes = read_raw_request(&mut retried).await;
        assert_eq!(
            retry_bytes, first_bytes,
            "ambiguous retry must be byte-identical"
        );
        let retry_frame = decode_raw_request(&retry_bytes);
        let retry: v1::SessionCreateRequest = retry_frame
            .decode_message(WireKind::SessionCreateRequest)
            .expect("decode retry mutation");
        executions += usize::from(
            seen.insert(
                retry
                    .operation_id
                    .as_ref()
                    .expect("retry operation ID")
                    .encode_to_vec(),
            ),
        );
        let response = encode_message(
            WireKind::SessionMutateResponse,
            retry_frame.request_id,
            0,
            &v1::SessionMutateResponse {
                session: Some(v1::SessionSummary {
                    session_id: Some(v1::SessionId { value: vec![8; 16] }),
                    name: retry.name,
                    revision: 0,
                    has_controller: false,
                    working_directory: temporary.path().to_string_lossy().into_owned(),
                    viewport: Some(TerminalSize::new(24, 80).into()),
                }),
            },
        )
        .expect("encode retained mutation result");
        retried
            .write_all(&response)
            .await
            .expect("write replay result");
        retried.shutdown().await.expect("finish replay result");
        executions
    });

    let client = LocalClient::new(&socket);
    let summary = client
        .create_session(
            &SessionName::new("exact-retry").expect("retry fixture name"),
            None,
            Some(TerminalSize::new(24, 80)),
        )
        .await
        .expect("ambiguous request retries once");
    assert_eq!(summary.name.as_str(), "exact-retry");
    assert_eq!(server.await.expect("retry fixture task"), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn typed_outcome_unknown_poison_rotates_only_on_the_next_user_operation() {
    let temporary = tempfile::tempdir().expect("temporary poisoned-lease fixture");
    let socket = temporary.path().join("poisoned.sock");
    let listener = tokio::net::UnixListener::bind(&socket).expect("bind poisoned fixture");
    let server = tokio::spawn(async move {
        for ordinal in [1_u64, 2] {
            let (mut lease_stream, _) = listener.accept().await.expect("accept lease request");
            let lease_frame = decode_raw_request(&read_raw_request(&mut lease_stream).await);
            assert_eq!(lease_frame.kind, WireKind::SessionOperationLeaseRequest);
            let response = encode_message(
                WireKind::SessionOperationLeaseResponse,
                lease_frame.request_id,
                0,
                &v1::SessionOperationLeaseResponse {
                    lease: Some(v1::OperationLease {
                        daemon_incarnation: vec![9; 16],
                        ordinal,
                    }),
                },
            )
            .expect("encode lease response");
            lease_stream
                .write_all(&response)
                .await
                .expect("write lease");
            lease_stream.shutdown().await.expect("finish lease");

            let (mut mutation_stream, _) = listener.accept().await.expect("accept mutation");
            let mutation_frame = decode_raw_request(&read_raw_request(&mut mutation_stream).await);
            let mutation: v1::SessionCreateRequest = mutation_frame
                .decode_message(WireKind::SessionCreateRequest)
                .expect("decode mutation");
            assert_eq!(
                mutation
                    .operation_id
                    .as_ref()
                    .expect("operation ID")
                    .lease_ordinal,
                ordinal
            );
            let response = if ordinal == 1 {
                encode_message(
                    WireKind::ServiceErrorResponse,
                    mutation_frame.request_id,
                    0,
                    &v1::ServiceError {
                        code: DomainErrorKind::OperationOutcomeUnknown.code().to_owned(),
                        message: "retired fixture lease".to_owned(),
                    },
                )
                .expect("encode outcome unknown")
            } else {
                encode_message(
                    WireKind::SessionMutateResponse,
                    mutation_frame.request_id,
                    0,
                    &v1::SessionMutateResponse {
                        session: Some(v1::SessionSummary {
                            session_id: Some(v1::SessionId { value: vec![6; 16] }),
                            name: mutation.name,
                            revision: 0,
                            has_controller: false,
                            working_directory: temporary.path().to_string_lossy().into_owned(),
                            viewport: Some(TerminalSize::new(24, 80).into()),
                        }),
                    },
                )
                .expect("encode successful independent mutation")
            };
            mutation_stream
                .write_all(&response)
                .await
                .expect("write mutation response");
            mutation_stream.shutdown().await.expect("finish mutation");
        }
    });

    let client = LocalClient::new(&socket);
    let first = client
        .create_session(
            &SessionName::new("poisoned-first").expect("fixture name"),
            None,
            Some(TerminalSize::new(24, 80)),
        )
        .await
        .expect_err("typed outcome unknown is returned without transport retry");
    assert_eq!(first.kind(), DomainErrorKind::OperationOutcomeUnknown);
    let second = client
        .create_session(
            &SessionName::new("independent-second").expect("fixture name"),
            None,
            Some(TerminalSize::new(24, 80)),
        )
        .await
        .expect("later independent mutation obtains a fresh lease");
    assert_eq!(second.name.as_str(), "independent-second");
    server.await.expect("poisoned lease fixture task");
}

#[cfg(unix)]
async fn assert_error_code(state: &TestState, request: Vec<u8>, expected: &str) {
    let mut stream = tokio::net::UnixStream::connect(state.paths.socket())
        .await
        .expect("raw client");
    stream.write_all(&request).await.expect("raw request");
    stream.shutdown().await.expect("finish raw request");
    let response = read_response(&mut stream).await;
    assert_service_error(&response, expected);
}

#[cfg(unix)]
async fn read_response(stream: &mut tokio::net::UnixStream) -> zterm_proto::DecodedFrame {
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .await
        .expect("response bytes");
    let mut decoder = FrameDecoder::new();
    let frames = decoder.feed(&bytes).expect("response frame");
    decoder.finish().expect("complete response");
    assert_eq!(frames.len(), 1);
    frames.into_iter().next().expect("one response")
}

#[cfg(unix)]
async fn read_raw_request(stream: &mut tokio::net::UnixStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .await
        .expect("raw request bytes");
    bytes
}

#[cfg(unix)]
fn decode_raw_request(bytes: &[u8]) -> zterm_proto::DecodedFrame {
    let mut decoder = FrameDecoder::new();
    let mut frames = decoder.feed(bytes).expect("raw request frame");
    decoder.finish().expect("complete raw request");
    assert_eq!(frames.len(), 1);
    frames.remove(0)
}

#[cfg(unix)]
fn assert_service_error(frame: &zterm_proto::DecodedFrame, expected: &str) {
    let error: v1::ServiceError = frame
        .decode_message(WireKind::ServiceErrorResponse)
        .expect("service error response");
    assert_eq!(error.code, expected);
}

#[cfg(unix)]
fn raw_wire(wire: v1::WireFrame) -> Vec<u8> {
    let body = wire.encode_to_vec();
    assert!(body.len() < 0x80);
    let mut bytes = vec![u8::try_from(body.len()).expect("small wire fixture")];
    bytes.extend_from_slice(&body);
    bytes
}
