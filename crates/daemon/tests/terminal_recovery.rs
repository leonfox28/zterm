//! Snapshot recovery and attachment-protocol fault isolation over the real Unix socket.

#![cfg(unix)]

#[path = "support/session_fixture.rs"]
mod session_fixture;
#[path = "support/state_fixture.rs"]
mod state_fixture;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zterm_core::{ResourceLimits, Revision, SessionName};
use zterm_daemon::bootstrap::bootstrap;
use zterm_daemon::config::{ValidatedInfrastructure, validate_setup_input};
use zterm_daemon::local_ipc::{
    LocalAttachmentClient, LocalAttachmentEvent, LocalClient, serve_local,
};
use zterm_daemon::service::DaemonService;
use zterm_platform::local_unix::{DaemonLock, bind_daemon_socket, remove_own_socket};
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v1};

use state_fixture::TestState;

const DEADLINE: Duration = Duration::from_secs(10);

#[tokio::test]
async fn future_snapshot_ack_recovers_and_wrong_kind_is_stream_local() -> Result<(), String> {
    let state = TestState::new();
    let requested = validate_setup_input("terminal-recovery", ValidatedInfrastructure::OfficialN0)
        .map_err(session_fixture::display)?;
    let setup = bootstrap(&state.paths, &requested).map_err(session_fixture::display)?;
    let lock = DaemonLock::try_acquire(&state.paths)
        .map_err(session_fixture::display)?
        .ok_or_else(|| "daemon lock unavailable".to_owned())?;
    let listener = bind_daemon_socket(&state.paths, &lock).map_err(session_fixture::display)?;
    let sessions = session_fixture::service_for_path(
        state.paths.home().to_path_buf(),
        ResourceLimits::default(),
    )?;
    let service = Arc::new(DaemonService::with_sessions(setup, 73, sessions));
    let server = tokio::spawn(serve_local(
        listener,
        state.paths.uid(),
        Arc::clone(&service),
    ));
    let unary = LocalClient::new(state.paths.socket());

    let mut attachment = LocalAttachmentClient::connect_main(state.paths.socket(), None)
        .await
        .map_err(session_fixture::display)?;
    let session_id = attachment.session_id();
    attachment
        .snapshot_applied(Revision::new(u64::MAX))
        .await
        .map_err(session_fixture::display)?;
    let LocalAttachmentEvent::SyncRequired(required) = attachment
        .read_event(DEADLINE)
        .await
        .map_err(session_fixture::display)?
    else {
        return Err("future snapshot acknowledgement did not require sync".into());
    };
    let LocalAttachmentEvent::Snapshot(replacement) = attachment
        .read_event(DEADLINE)
        .await
        .map_err(session_fixture::display)?
    else {
        return Err("sync requirement was not followed by a replacement snapshot".into());
    };
    assert_eq!(required.latest_revision, replacement.revision);
    attachment
        .detach()
        .await
        .map_err(session_fixture::display)?;
    drop(attachment);
    wait_for_controller_release(&unary).await?;

    let mut raw = tokio::net::UnixStream::connect(state.paths.socket())
        .await
        .map_err(session_fixture::display)?;
    let attach = encode_message(
        WireKind::TerminalAttachRequest,
        90,
        0,
        &v1::TerminalAttachRequest {
            target: Some(v1::TargetSelector {
                target: Some(v1::target_selector::Target::Local(true)),
            }),
            session_id: Some(session_id.into()),
            takeover: false,
            session_name: String::new(),
            create_main: false,
            viewport: None,
        },
    )
    .map_err(session_fixture::display)?;
    raw.write_all(&attach)
        .await
        .map_err(session_fixture::display)?;
    let mut decoder = FrameDecoder::new();
    let mut queued = VecDeque::new();
    let initial = read_frame(&mut raw, &mut decoder, &mut queued).await?;
    let _: v1::TerminalSnapshot = initial
        .decode_message(WireKind::TerminalSnapshot)
        .map_err(session_fixture::display)?;

    let wrong_kind = encode_message(
        WireKind::LocalStatusRequest,
        91,
        0,
        &v1::LocalStatusRequest {},
    )
    .map_err(session_fixture::display)?;
    raw.write_all(&wrong_kind)
        .await
        .map_err(session_fixture::display)?;
    let error_frame = read_frame(&mut raw, &mut decoder, &mut queued).await?;
    let error: v1::ServiceError = error_frame
        .decode_message(WireKind::ServiceErrorResponse)
        .map_err(session_fixture::display)?;
    assert_eq!(error.code, "malformed_frame");
    drop(raw);
    wait_for_controller_release(&unary).await?;

    let mut oversized = tokio::net::UnixStream::connect(state.paths.socket())
        .await
        .map_err(session_fixture::display)?;
    oversized
        .write_all(&attach)
        .await
        .map_err(session_fixture::display)?;
    let mut oversized_decoder = FrameDecoder::new();
    let mut oversized_queued = VecDeque::new();
    let initial = read_frame(
        &mut oversized,
        &mut oversized_decoder,
        &mut oversized_queued,
    )
    .await?;
    let _: v1::TerminalSnapshot = initial
        .decode_message(WireKind::TerminalSnapshot)
        .map_err(session_fixture::display)?;
    oversized
        .write_all(&[0x81, 0x80, 0x80, 0x04])
        .await
        .map_err(session_fixture::display)?;
    let error_frame = read_frame(
        &mut oversized,
        &mut oversized_decoder,
        &mut oversized_queued,
    )
    .await?;
    let error: v1::ServiceError = error_frame
        .decode_message(WireKind::ServiceErrorResponse)
        .map_err(session_fixture::display)?;
    assert_eq!(error.code, "frame_too_large");
    drop(oversized);
    wait_for_controller_release(&unary).await?;

    let other = unary
        .create_session(
            &SessionName::new("still-live").map_err(session_fixture::display)?,
            None,
            None,
        )
        .await
        .map_err(session_fixture::display)?;
    let sessions = unary
        .list_sessions()
        .await
        .map_err(session_fixture::display)?;
    assert!(
        sessions
            .iter()
            .any(|session| session.session_id == session_id)
    );
    assert!(
        sessions
            .iter()
            .any(|session| session.session_id == other.session_id)
    );

    let stopped = unary.stop(false).await.map_err(session_fixture::display)?;
    assert_eq!(stopped.active_session_count, 2);
    server
        .await
        .map_err(session_fixture::display)?
        .map_err(session_fixture::display)?;
    remove_own_socket(&state.paths, &lock).map_err(session_fixture::display)?;
    Ok(())
}

async fn wait_for_controller_release(client: &LocalClient) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let sessions = client
            .list_sessions()
            .await
            .map_err(session_fixture::display)?;
        if sessions.iter().all(|session| !session.has_controller) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("detached controller was not released".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn read_frame(
    stream: &mut tokio::net::UnixStream,
    decoder: &mut FrameDecoder,
    queued: &mut VecDeque<DecodedFrame>,
) -> Result<DecodedFrame, String> {
    if let Some(frame) = queued.pop_front() {
        return Ok(frame);
    }
    let deadline = Instant::now() + DEADLINE;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            stream.read(&mut buffer),
        )
        .await
        .map_err(session_fixture::display)?
        .map_err(session_fixture::display)?;
        if read == 0 {
            return Err("terminal socket closed before a complete frame".into());
        }
        queued.extend(
            decoder
                .feed(&buffer[..read])
                .map_err(session_fixture::display)?,
        );
        if let Some(frame) = queued.pop_front() {
            return Ok(frame);
        }
    }
}
