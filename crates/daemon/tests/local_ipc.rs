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
use zterm_core::DomainErrorKind;
#[cfg(unix)]
use zterm_daemon::bootstrap::bootstrap;
#[cfg(unix)]
use zterm_daemon::config::{ValidatedInfrastructure, validate_setup_input};
#[cfg(unix)]
use zterm_daemon::local_ipc::{LocalClient, LocalIpcLimits, serve_local_with_limits};
#[cfg(unix)]
use zterm_daemon::service::DaemonService;
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

    let future = encode_message(WireKind::PairOffer, 13, 0, &v1::PairOffer::default())
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
