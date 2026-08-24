//! Real same-UID Unix socket coverage for session unary and duplex attachment traffic.

#![cfg(unix)]

#[path = "support/session_fixture.rs"]
mod session_fixture;
#[path = "support/state_fixture.rs"]
mod state_fixture;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zterm_core::terminal::TerminalSize;
use zterm_core::{
    AttachmentId, DeviceId, DomainErrorKind, OperationId, ResourceLimits, Revision, SessionName,
    SessionSelector,
};
use zterm_daemon::bootstrap::bootstrap;
use zterm_daemon::config::{ValidatedInfrastructure, validate_setup_input};
use zterm_daemon::lifecycle::run_owned_daemon_listener_for_test;
use zterm_daemon::local_ipc::{
    LocalAttachmentClient, LocalAttachmentEvent, LocalClient, LocalIpcLimits, serve_local,
    serve_local_with_limits,
};
use zterm_daemon::service::DaemonService;
use zterm_platform::local_unix::{
    DaemonLock, bind_daemon_socket, bind_owned_daemon_socket, remove_own_socket,
};
use zterm_platform::pty::{ExplicitPtyCommand, PtyHost, PtySize};
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v1};

use state_fixture::TestState;

const EVENT_DEADLINE: Duration = Duration::from_secs(10);

#[tokio::test]
async fn unary_mutations_and_duplex_reconnect_share_one_live_registry() -> Result<(), String> {
    let state = TestState::new();
    let requested = validate_setup_input("session-ipc", ValidatedInfrastructure::OfficialN0)
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
    let service = Arc::new(DaemonService::with_sessions(setup, 41, sessions));
    let server = tokio::spawn(serve_local(
        listener,
        state.paths.uid(),
        Arc::clone(&service),
    ));
    let client = LocalClient::new(state.paths.socket());

    let build_name = SessionName::new("build").map_err(session_fixture::display)?;
    let build = client
        .create_session(&build_name, None, Some(TerminalSize::new(24, 80)))
        .await
        .map_err(session_fixture::display)?;
    assert_eq!(
        client
            .list_sessions()
            .await
            .map_err(session_fixture::display)?
            .len(),
        1
    );
    let renamed_name = SessionName::new("review").map_err(session_fixture::display)?;
    let renamed = client
        .rename_session(build.session_id, &renamed_name)
        .await
        .map_err(session_fixture::display)?;
    assert_eq!(renamed.session_id, build.session_id);
    assert_eq!(renamed.name, renamed_name);
    let closed = client
        .close_session(build.session_id)
        .await
        .map_err(session_fixture::display)?;
    assert_eq!(closed.session_id, build.session_id);

    let mut attached =
        LocalAttachmentClient::connect_main(state.paths.socket(), Some(TerminalSize::new(30, 100)))
            .await
            .map_err(session_fixture::display)?;
    let session_id = attached.session_id();
    synchronize(&mut attached)
        .await
        .map_err(|error| format!("initial attachment synchronization failed: {error}"))?;
    attached
        .write_input(b"printf 'SOCKET-RECONNECT-MARKER\\n'\n".to_vec())
        .await
        .map_err(session_fixture::display)?;
    wait_for_wire_text(&mut attached, b"SOCKET-RECONNECT-MARKER")
        .await
        .map_err(|error| format!("initial attachment output failed: {error}"))?;
    attached.detach().await.map_err(session_fixture::display)?;

    let listed = client
        .list_sessions()
        .await
        .map_err(session_fixture::display)?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].session_id, session_id);
    assert_eq!(listed[0].name, SessionName::main());
    let status = client.status().await.map_err(session_fixture::display)?;
    assert_eq!(status.active_session_count, 1);
    assert_eq!(status.active_session_names, ["main"]);

    let mut reattached = LocalAttachmentClient::connect_session(
        state.paths.socket(),
        SessionSelector::Id(session_id),
        false,
        None,
    )
    .await
    .map_err(session_fixture::display)?;
    assert_eq!(reattached.session_id(), session_id);
    assert!(snapshot_contains(
        reattached.initial_snapshot(),
        b"SOCKET-RECONNECT-MARKER"
    ));

    synchronize(&mut reattached)
        .await
        .map_err(|error| format!("reattachment synchronization failed: {error}"))?;
    reattached
        .write_input(b"printf 'SOCKET-FINAL-MARKER\\n'; exit\n".to_vec())
        .await
        .map_err(session_fixture::display)?;
    let mut saw_final_output = false;
    loop {
        match reattached
            .read_event(EVENT_DEADLINE)
            .await
            .map_err(|error| format!("natural-exit attachment event failed: {error}"))?
        {
            LocalAttachmentEvent::Delta(delta) => {
                saw_final_output |= contains(&delta.ansi, b"SOCKET-FINAL-MARKER");
            }
            LocalAttachmentEvent::Snapshot(snapshot) => {
                saw_final_output |= snapshot_contains(&snapshot, b"SOCKET-FINAL-MARKER");
            }
            LocalAttachmentEvent::SyncRequired(_) => {}
            LocalAttachmentEvent::TransportState(_) => {}
            LocalAttachmentEvent::SessionEnded(ended) => {
                assert_eq!(
                    ended.reason,
                    zterm_proto::v1::TerminalSessionEndReason::NaturalExit as i32
                );
                break;
            }
            LocalAttachmentEvent::LeaseLost(_) => {
                return Err("controller lease was lost before natural exit".into());
            }
            LocalAttachmentEvent::Takeover(_) => {
                return Err("unexpected takeover response before natural exit".into());
            }
        }
    }
    assert!(
        saw_final_output,
        "the final drained terminal bytes must precede SessionEnded"
    );

    let mut stop_attachment =
        LocalAttachmentClient::connect_main(state.paths.socket(), Some(TerminalSize::new(30, 100)))
            .await
            .map_err(session_fixture::display)?;
    let stopped = client.stop(false).await.map_err(session_fixture::display)?;
    assert_eq!(stopped.active_session_count, 1);
    assert_eq!(stopped.active_session_names, ["main"]);
    let stop_deadline = Instant::now() + EVENT_DEADLINE;
    let ended = loop {
        let remaining = stop_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("daemon stop did not terminate the attached session explicitly".into());
        }
        match stop_attachment
            .read_event(remaining)
            .await
            .map_err(|error| format!("daemon-stop attachment event failed: {error}"))?
        {
            LocalAttachmentEvent::Snapshot(_)
            | LocalAttachmentEvent::Delta(_)
            | LocalAttachmentEvent::SyncRequired(_)
            | LocalAttachmentEvent::TransportState(_) => {}
            LocalAttachmentEvent::SessionEnded(ended) => break ended,
            LocalAttachmentEvent::LeaseLost(_) => {
                return Err("controller lease was lost during daemon stop".into());
            }
            LocalAttachmentEvent::Takeover(_) => {
                return Err("unexpected takeover response during daemon stop".into());
            }
        }
    };
    assert_eq!(
        ended.reason,
        zterm_proto::v1::TerminalSessionEndReason::DaemonStop as i32
    );
    server
        .await
        .map_err(session_fixture::display)?
        .map_err(session_fixture::display)?;
    remove_own_socket(&state.paths, &lock).map_err(session_fixture::display)?;
    Ok(())
}

#[tokio::test]
async fn mutation_response_loss_replays_the_exact_completed_result_on_a_new_socket()
-> Result<(), String> {
    let state = TestState::new();
    let requested = validate_setup_input("response-loss", ValidatedInfrastructure::OfficialN0)
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
    let service = Arc::new(DaemonService::with_sessions(setup, 42, sessions));
    let server = tokio::spawn(serve_local(
        listener,
        state.paths.uid(),
        Arc::clone(&service),
    ));

    let lease_request = encode_message(
        WireKind::SessionOperationLeaseRequest,
        89,
        5_000,
        &v1::SessionOperationLeaseRequest {
            target: Some(local_target()),
        },
    )
    .map_err(session_fixture::display)?;
    let lease_frame = raw_unary(state.paths.socket(), &lease_request).await?;
    let lease: zterm_core::OperationLease = lease_frame
        .decode_message::<v1::SessionOperationLeaseResponse>(
            WireKind::SessionOperationLeaseResponse,
        )
        .map_err(session_fixture::display)?
        .lease
        .ok_or_else(|| "lease response omitted lease".to_owned())?
        .try_into()
        .map_err(session_fixture::display)?;
    let operation_id: v1::OperationId = zterm_core::OperationId { lease, sequence: 9 }.into();
    let first_request = session_create_bytes(90, operation_id.clone(), "response-loss")?;
    let mut abandoned = tokio::net::UnixStream::connect(state.paths.socket())
        .await
        .map_err(session_fixture::display)?;
    abandoned
        .write_all(&first_request)
        .await
        .map_err(session_fixture::display)?;
    abandoned
        .shutdown()
        .await
        .map_err(session_fixture::display)?;
    drop(abandoned);

    let client = LocalClient::new(state.paths.socket());
    let deadline = Instant::now() + EVENT_DEADLINE;
    let completed = loop {
        let listed = client
            .list_sessions()
            .await
            .map_err(session_fixture::display)?;
        if let Some(summary) = listed.into_iter().next() {
            break summary;
        }
        if Instant::now() >= deadline {
            return Err("the abandoned request never completed its accepted create".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(completed.name.as_str(), "response-loss");

    let retry = session_create_bytes(91, operation_id.clone(), "response-loss")?;
    let frame = raw_unary(state.paths.socket(), &retry).await?;
    assert_eq!(frame.kind, WireKind::SessionMutateResponse);
    let response: v1::SessionMutateResponse = frame
        .decode_message(WireKind::SessionMutateResponse)
        .map_err(session_fixture::display)?;
    let replayed = response
        .session
        .ok_or_else(|| "replayed mutation omitted its session".to_owned())?;
    assert_eq!(replayed.name, "response-loss");
    assert_eq!(
        replayed
            .session_id
            .ok_or_else(|| "replayed mutation omitted its session id".to_owned())?
            .value,
        completed.session_id.as_bytes()
    );
    assert_eq!(
        client
            .list_sessions()
            .await
            .map_err(session_fixture::display)?
            .len(),
        1
    );

    let mismatch = session_create_bytes(92, operation_id, "must-not-run")?;
    let mismatch = raw_unary(state.paths.socket(), &mismatch).await?;
    assert_eq!(mismatch.kind, WireKind::ServiceErrorResponse);
    let mismatch: v1::ServiceError = mismatch
        .decode_message(WireKind::ServiceErrorResponse)
        .map_err(session_fixture::display)?;
    assert_eq!(
        mismatch.code,
        DomainErrorKind::OperationOutcomeUnknown.code()
    );

    client.stop(false).await.map_err(session_fixture::display)?;
    server
        .await
        .map_err(session_fixture::display)?
        .map_err(session_fixture::display)?;
    remove_own_socket(&state.paths, &lock).map_err(session_fixture::display)?;
    Ok(())
}

#[tokio::test]
async fn takeover_response_loss_reconnects_with_input_authority_while_old_stream_is_live()
-> Result<(), String> {
    let state = TestState::new();
    let requested = validate_setup_input("takeover-retry", ValidatedInfrastructure::OfficialN0)
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
    let service = Arc::new(DaemonService::with_sessions(setup, 45, sessions));
    let server = tokio::spawn(serve_local(
        listener,
        state.paths.uid(),
        Arc::clone(&service),
    ));
    let client = LocalClient::new(state.paths.socket());

    let mut original =
        LocalAttachmentClient::connect_main(state.paths.socket(), Some(TerminalSize::new(24, 80)))
            .await
            .map_err(session_fixture::display)?;
    synchronize(&mut original).await?;
    let session_id = original.session_id();

    let mut ambiguous = LocalAttachmentClient::connect_session(
        state.paths.socket(),
        SessionSelector::Id(session_id),
        true,
        None,
    )
    .await
    .map_err(session_fixture::display)?;
    synchronize(&mut ambiguous).await?;
    let retry = ambiguous
        .begin_takeover()
        .await
        .map_err(session_fixture::display)?;
    wait_for_lease_lost(&mut original).await?;
    // Do not read the mutate response from `ambiguous`: from the client's
    // perspective its outcome is lost while the old stream remains attached.

    let mut replacement = LocalAttachmentClient::connect_session(
        state.paths.socket(),
        SessionSelector::Id(session_id),
        true,
        None,
    )
    .await
    .map_err(session_fixture::display)?;
    synchronize(&mut replacement).await?;
    replacement
        .retry_takeover(retry)
        .await
        .map_err(session_fixture::display)?;
    loop {
        match replacement
            .read_event(EVENT_DEADLINE)
            .await
            .map_err(session_fixture::display)?
        {
            LocalAttachmentEvent::Takeover(summary) => {
                assert_eq!(summary.session_id, session_id);
                break;
            }
            LocalAttachmentEvent::Delta(_)
            | LocalAttachmentEvent::Snapshot(_)
            | LocalAttachmentEvent::SyncRequired(_) => {}
            event => return Err(format!("unexpected takeover continuation event: {event:?}")),
        }
    }
    wait_for_lease_lost(&mut ambiguous).await?;
    replacement
        .write_input(b"printf 'TAKEOVER-RETRY-AUTHORITY\\n'\n".to_vec())
        .await
        .map_err(session_fixture::display)?;
    wait_for_wire_text(&mut replacement, b"TAKEOVER-RETRY-AUTHORITY").await?;

    client.stop(false).await.map_err(session_fixture::display)?;
    server
        .await
        .map_err(session_fixture::display)?
        .map_err(session_fixture::display)?;
    remove_own_socket(&state.paths, &lock).map_err(session_fixture::display)?;
    Ok(())
}

#[tokio::test]
async fn blocked_pty_input_does_not_stall_the_socket_runtime_or_an_unrelated_session()
-> Result<(), String> {
    let state = TestState::new();
    let requested = validate_setup_input("blocked-pty", ValidatedInfrastructure::OfficialN0)
        .map_err(session_fixture::display)?;
    let setup = bootstrap(&state.paths, &requested).map_err(session_fixture::display)?;
    let lock = DaemonLock::try_acquire(&state.paths)
        .map_err(session_fixture::display)?
        .ok_or_else(|| "daemon lock unavailable".to_owned())?;
    let listener = bind_daemon_socket(&state.paths, &lock).map_err(session_fixture::display)?;
    let shell = shell_path()?;
    let cwd = state.paths.home().to_path_buf();
    let spawn_count = Arc::new(AtomicUsize::new(0));
    let sessions = zterm_daemon::session::SessionService::with_spawner(
        DeviceId::from_array([44; 32]),
        ResourceLimits::default(),
        {
            let shell = shell.clone();
            let cwd = cwd.clone();
            let spawn_count = Arc::clone(&spawn_count);
            move |size, requested| {
                let working_directory = requested.unwrap_or(&cwd).to_path_buf();
                let command = if spawn_count.fetch_add(1, Ordering::AcqRel) == 0 {
                    ExplicitPtyCommand::new(&shell, &working_directory)
                        .arg("-c")
                        .arg(
                            "printf 'BLOCK-READY\\r\\n'; stty -echo; trap '' HUP; while :; do sleep 1; done",
                        )
                } else {
                    ExplicitPtyCommand::new(&shell, &working_directory).arg("-i")
                };
                let session = PtyHost::new()
                    .spawn(command, PtySize::new(size.rows, size.columns))
                    .map_err(|error| {
                        zterm_daemon::error::DaemonError::new(
                            DomainErrorKind::StoreUnavailable,
                            error.to_string(),
                        )
                    })?;
                Ok((session, working_directory))
            }
        },
    );
    let service = Arc::new(DaemonService::with_sessions(setup, 43, sessions));
    let server = tokio::spawn(serve_local(
        listener,
        state.paths.uid(),
        Arc::clone(&service),
    ));
    let client = LocalClient::new(state.paths.socket());
    let mut blocked =
        LocalAttachmentClient::connect_main(state.paths.socket(), Some(TerminalSize::new(24, 80)))
            .await
            .map_err(session_fixture::display)?;
    let ready_in_initial = snapshot_contains(blocked.initial_snapshot(), b"BLOCK-READY");
    synchronize(&mut blocked).await?;
    if !ready_in_initial {
        blocked
            .request_sync(Revision::ZERO)
            .await
            .map_err(session_fixture::display)?;
        wait_for_wire_text(&mut blocked, b"BLOCK-READY").await?;
    }

    blocked
        .write_input(vec![b'x'; 900 * 1024])
        .await
        .map_err(session_fixture::display)?;
    tokio::time::sleep(Duration::from_millis(75)).await;

    let status = tokio::time::timeout(Duration::from_secs(1), client.status())
        .await
        .map_err(|_| "status stalled behind another session's blocked PTY write".to_owned())?
        .map_err(session_fixture::display)?;
    assert_eq!(status.active_session_count, 1);

    let fast_name = SessionName::new("fast-while-blocked").map_err(session_fixture::display)?;
    let fast = tokio::time::timeout(
        Duration::from_secs(1),
        client.create_session(&fast_name, None, Some(TerminalSize::new(24, 80))),
    )
    .await
    .map_err(|_| "session B creation stalled behind session A's PTY write".to_owned())?
    .map_err(session_fixture::display)?;
    assert_eq!(fast.name, fast_name);

    tokio::time::timeout(
        Duration::from_secs(3),
        client.close_session(blocked.session_id()),
    )
    .await
    .map_err(|_| "independent child interruption did not release the blocked PTY owner".to_owned())?
    .map_err(session_fixture::display)?;
    client.stop(false).await.map_err(session_fixture::display)?;
    server
        .await
        .map_err(session_fixture::display)?
        .map_err(session_fixture::display)?;
    remove_own_socket(&state.paths, &lock).map_err(session_fixture::display)?;
    Ok(())
}

#[tokio::test]
async fn failed_bounded_stop_keeps_the_listener_available_until_session_ownership_is_released()
-> Result<(), String> {
    let state = TestState::new();
    let requested = validate_setup_input("truthful-stop", ValidatedInfrastructure::OfficialN0)
        .map_err(session_fixture::display)?;
    let setup = bootstrap(&state.paths, &requested).map_err(session_fixture::display)?;
    let lock = DaemonLock::try_acquire(&state.paths)
        .map_err(session_fixture::display)?
        .ok_or_else(|| "daemon lock unavailable".to_owned())?;
    let listener = bind_daemon_socket(&state.paths, &lock).map_err(session_fixture::display)?;
    let shell = shell_path()?;
    let cwd = state.paths.home().to_path_buf();
    let sessions = zterm_daemon::session::SessionService::with_spawner(
        DeviceId::from_array([45; 32]),
        ResourceLimits::default(),
        move |size, requested| {
            let working_directory = requested.unwrap_or(&cwd).to_path_buf();
            let session = PtyHost::new()
                .spawn(
                    ExplicitPtyCommand::new(&shell, &working_directory)
                        .arg("-c")
                        .arg("trap '' HUP; printf 'STOP-READY\\r\\n'; while :; do :; done"),
                    PtySize::new(size.rows, size.columns),
                )
                .map_err(|error| {
                    zterm_daemon::error::DaemonError::new(
                        DomainErrorKind::StoreUnavailable,
                        error.to_string(),
                    )
                })?;
            Ok((session, working_directory))
        },
    );
    let principal = sessions.local_principal(AttachmentId::from_array([46; 16]));
    let prepared = sessions
        .prepare_attach(
            principal,
            None,
            true,
            false,
            Some(TerminalSize::new(24, 80)),
        )
        .map_err(session_fixture::display)?;
    let ready_deadline = Instant::now() + EVENT_DEADLINE;
    loop {
        let snapshot = prepared
            .attachment
            .sync_latest(Revision::ZERO)
            .map_err(session_fixture::display)?;
        if snapshot_contains_domain(&snapshot, b"STOP-READY") {
            break;
        }
        if Instant::now() >= ready_deadline {
            return Err("HUP-resistant stop fixture never became ready".into());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let service = Arc::new(DaemonService::with_sessions(setup, 44, sessions.clone()));
    let server = tokio::spawn(serve_local_with_limits(
        listener,
        state.paths.uid(),
        Arc::clone(&service),
        LocalIpcLimits::for_test(Duration::from_millis(40)),
    ));
    let client = LocalClient::new(state.paths.socket());
    let failed_stop = client
        .stop(false)
        .await
        .expect_err("the daemon cannot acknowledge stop before cleanup finishes");
    assert_eq!(failed_stop.kind(), DomainErrorKind::DeadlineExceeded);

    let status = client.status().await.map_err(session_fixture::display)?;
    assert_eq!(status.active_session_count, 1);
    let completion_deadline = Instant::now() + Duration::from_secs(3);
    while !sessions
        .list()
        .map_err(session_fixture::display)?
        .is_empty()
        && Instant::now() < completion_deadline
    {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        sessions
            .list()
            .map_err(session_fixture::display)?
            .is_empty()
    );

    let stopped = client.stop(false).await.map_err(session_fixture::display)?;
    assert!(stopped.stopping);
    server
        .await
        .map_err(session_fixture::display)?
        .map_err(session_fixture::display)?;
    remove_own_socket(&state.paths, &lock).map_err(session_fixture::display)?;
    Ok(())
}

#[tokio::test]
async fn listener_accept_failure_preserves_live_session_ownership_and_retryability()
-> Result<(), String> {
    let state = TestState::new();
    let requested = validate_setup_input("accept-retry", ValidatedInfrastructure::OfficialN0)
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
    let service = Arc::new(DaemonService::with_sessions(setup, 46, sessions));
    let server = tokio::spawn(serve_local_with_limits(
        listener,
        state.paths.uid(),
        Arc::clone(&service),
        LocalIpcLimits::for_test(Duration::from_secs(2)).with_accept_failure_after_for_test(1),
    ));
    let mut attached =
        LocalAttachmentClient::connect_main(state.paths.socket(), Some(TerminalSize::new(24, 80)))
            .await
            .map_err(session_fixture::display)?;
    synchronize(&mut attached).await?;

    let client = LocalClient::new(state.paths.socket());
    let status = client.status().await.map_err(session_fixture::display)?;
    assert_eq!(status.active_session_count, 1);
    assert!(
        !server.is_finished(),
        "accept failure terminated the owner task"
    );
    attached
        .write_input(b"printf 'ACCEPT-RETRY-OWNER\\n'\n".to_vec())
        .await
        .map_err(session_fixture::display)?;
    wait_for_wire_text(&mut attached, b"ACCEPT-RETRY-OWNER").await?;

    client.stop(false).await.map_err(session_fixture::display)?;
    server
        .await
        .map_err(session_fixture::display)?
        .map_err(session_fixture::display)?;
    remove_own_socket(&state.paths, &lock).map_err(session_fixture::display)?;
    Ok(())
}

#[tokio::test]
async fn fatal_listener_exit_rebinds_actual_daemon_loop_until_owned_child_can_stop()
-> Result<(), String> {
    let state = TestState::new();
    let requested = validate_setup_input(
        "fatal-listener-recovery",
        ValidatedInfrastructure::OfficialN0,
    )
    .map_err(session_fixture::display)?;
    let setup = bootstrap(&state.paths, &requested).map_err(session_fixture::display)?;
    let lock = DaemonLock::try_acquire(&state.paths)
        .map_err(session_fixture::display)?
        .ok_or_else(|| "daemon lock unavailable".to_owned())?;
    let (listener, socket_ownership) =
        bind_owned_daemon_socket(&state.paths, &lock).map_err(session_fixture::display)?;
    let shell = shell_path()?;
    let cwd = state.paths.home().to_path_buf();
    let process_id = Arc::new(Mutex::new(None));
    let sessions = zterm_daemon::session::SessionService::with_spawner(
        DeviceId::from_array([47; 32]),
        ResourceLimits::default(),
        {
            let process_id = Arc::clone(&process_id);
            move |size, requested| {
                let working_directory = requested.unwrap_or(&cwd).to_path_buf();
                let session = PtyHost::new()
                    .spawn(
                        ExplicitPtyCommand::new(&shell, &working_directory)
                            .arg("-c")
                            .arg("trap '' HUP; printf 'FATAL-OWNER-READY\\r\\n'; while :; do :; done"),
                        PtySize::new(size.rows, size.columns),
                    )
                    .map_err(|error| {
                        zterm_daemon::error::DaemonError::new(
                            DomainErrorKind::InvalidWorkingDirectory,
                            error.to_string(),
                        )
                    })?;
                *process_id.lock().map_err(|_| {
                    zterm_daemon::error::DaemonError::new(
                        DomainErrorKind::StoreUnavailable,
                        "fatal-listener fixture process lock poisoned",
                    )
                })? = session.process_id();
                Ok((session, working_directory))
            }
        },
    );
    let principal = sessions.local_principal(AttachmentId::from_array([47; 16]));
    let lease = sessions
        .issue_operation_lease(principal)
        .map_err(session_fixture::display)?;
    let created = sessions
        .create(
            principal,
            OperationId { lease, sequence: 1 },
            SessionName::new("fatal-owner").map_err(session_fixture::display)?,
            None,
            Some(TerminalSize::new(24, 80)),
        )
        .map_err(session_fixture::display)?;
    let ready = sessions
        .prepare_attach(
            principal,
            Some(SessionSelector::Id(created.session_id)),
            false,
            false,
            None,
        )
        .map_err(session_fixture::display)?;
    session_fixture::activate(&ready)?;
    session_fixture::wait_for_text(&ready.attachment, "FATAL-OWNER-READY")?;

    let service = Arc::new(DaemonService::with_sessions(setup, 47, sessions));
    let daemon_paths = state.paths.clone();
    let daemon_service = Arc::clone(&service);
    let server = std::thread::spawn(move || {
        run_owned_daemon_listener_for_test(
            &daemon_paths,
            lock,
            listener,
            socket_ownership,
            daemon_service,
            LocalIpcLimits::for_test(Duration::from_secs(2))
                .with_fatal_accept_failure_after_for_test(0),
            Duration::from_millis(20),
        )
    });

    let client = LocalClient::new(state.paths.socket());
    let recovery_deadline = Instant::now() + Duration::from_secs(1);
    let status = loop {
        match client.status().await {
            Ok(status) => break status,
            Err(error)
                if matches!(
                    error.kind(),
                    DomainErrorKind::DaemonStopped | DomainErrorKind::Cancelled
                ) =>
            {
                if Instant::now() >= recovery_deadline {
                    return Err("fatal listener never rebound its diagnostic socket".into());
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(error) => return Err(session_fixture::display(error)),
        }
    };
    assert_eq!(status.active_session_count, 1);
    assert_eq!(status.active_session_names, vec!["fatal-owner"]);
    assert!(
        !server.is_finished(),
        "daemon exited while child ownership remained"
    );
    let process_id = process_id
        .lock()
        .map_err(|_| "process lock poisoned".to_owned())?
        .ok_or_else(|| "fixture process ID missing".to_owned())?;
    assert!(
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(
                i32::try_from(process_id).map_err(session_fixture::display)?
            ),
            None,
        )
        .is_ok(),
        "recovered status must not outlive the retained child"
    );

    let stopped = client.stop(false).await.map_err(session_fixture::display)?;
    assert!(stopped.stopping);
    tokio::task::spawn_blocking(move || server.join())
        .await
        .map_err(session_fixture::display)?
        .map_err(|_| "owned daemon listener thread panicked".to_owned())?
        .map_err(session_fixture::display)?;
    assert!(
        !state.paths.socket().exists(),
        "successful final stop owns socket removal"
    );
    Ok(())
}

fn session_create_bytes(
    request_id: u64,
    operation_id: v1::OperationId,
    name: &str,
) -> Result<Vec<u8>, String> {
    encode_message(
        WireKind::SessionCreateRequest,
        request_id,
        5_000,
        &v1::SessionCreateRequest {
            operation_id: Some(operation_id),
            target: Some(v1::TargetSelector {
                target: Some(v1::target_selector::Target::Local(true)),
            }),
            name: name.to_owned(),
            working_directory: String::new(),
            viewport: Some(TerminalSize::new(24, 80).into()),
        },
    )
    .map_err(session_fixture::display)
}

fn local_target() -> v1::TargetSelector {
    v1::TargetSelector {
        target: Some(v1::target_selector::Target::Local(true)),
    }
}

async fn raw_unary(path: &std::path::Path, bytes: &[u8]) -> Result<DecodedFrame, String> {
    let mut stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(session_fixture::display)?;
    stream
        .write_all(bytes)
        .await
        .map_err(session_fixture::display)?;
    stream.shutdown().await.map_err(session_fixture::display)?;
    let mut decoder = FrameDecoder::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = tokio::time::timeout(EVENT_DEADLINE, stream.read(&mut buffer))
            .await
            .map_err(|_| "timed out waiting for raw unary response".to_owned())?
            .map_err(session_fixture::display)?;
        if read == 0 {
            decoder.finish().map_err(session_fixture::display)?;
            return Err("raw unary response ended without a frame".into());
        }
        let mut frames = decoder
            .feed(&buffer[..read])
            .map_err(session_fixture::display)?;
        if let Some(frame) = frames.pop() {
            return Ok(frame);
        }
    }
}

fn shell_path() -> Result<PathBuf, String> {
    [Path::new("/bin/sh"), Path::new("/usr/bin/sh")]
        .into_iter()
        .find(|path| path.is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| "no absolute POSIX shell fixture is available".to_owned())
}

async fn synchronize(client: &mut LocalAttachmentClient) -> Result<(), String> {
    let mut revision = Revision::new(client.initial_snapshot().revision);
    loop {
        client
            .snapshot_applied(revision)
            .await
            .map_err(session_fixture::display)?;
        match client.read_event(Duration::from_millis(100)).await {
            Ok(LocalAttachmentEvent::SyncRequired(_)) => {
                let LocalAttachmentEvent::Snapshot(snapshot) = client
                    .read_event(EVENT_DEADLINE)
                    .await
                    .map_err(session_fixture::display)?
                else {
                    return Err("sync-required was not followed by a snapshot".into());
                };
                revision = Revision::new(snapshot.revision);
            }
            Ok(LocalAttachmentEvent::Delta(_)) => return Ok(()),
            Err(error) if error.kind() == zterm_core::DomainErrorKind::DeadlineExceeded => {
                return Ok(());
            }
            Ok(event) => return Err(format!("unexpected event while synchronizing: {event:?}")),
            Err(error) => return Err(error.to_string()),
        }
    }
}

async fn wait_for_wire_text(
    client: &mut LocalAttachmentClient,
    needle: &[u8],
) -> Result<(), String> {
    let deadline = Instant::now() + EVENT_DEADLINE;
    while Instant::now() < deadline {
        match client
            .read_event(deadline.saturating_duration_since(Instant::now()))
            .await
            .map_err(session_fixture::display)?
        {
            LocalAttachmentEvent::Delta(delta) if contains(&delta.ansi, needle) => return Ok(()),
            LocalAttachmentEvent::Snapshot(snapshot) if snapshot_contains(&snapshot, needle) => {
                return Ok(());
            }
            LocalAttachmentEvent::SyncRequired(_) => {}
            event => {
                if matches!(
                    event,
                    LocalAttachmentEvent::LeaseLost(_) | LocalAttachmentEvent::SessionEnded(_)
                ) {
                    return Err(format!("terminal ended before marker: {event:?}"));
                }
            }
        }
    }
    Err("terminal stream did not contain reconnect marker".into())
}

async fn wait_for_lease_lost(client: &mut LocalAttachmentClient) -> Result<(), String> {
    loop {
        match client
            .read_event(EVENT_DEADLINE)
            .await
            .map_err(session_fixture::display)?
        {
            LocalAttachmentEvent::LeaseLost(_) => return Ok(()),
            LocalAttachmentEvent::Delta(_)
            | LocalAttachmentEvent::Snapshot(_)
            | LocalAttachmentEvent::SyncRequired(_)
            | LocalAttachmentEvent::TransportState(_)
            | LocalAttachmentEvent::Takeover(_) => {}
            LocalAttachmentEvent::SessionEnded(_) => {
                return Err("session ended while waiting for controller lease loss".into());
            }
        }
    }
}

fn snapshot_contains(snapshot: &zterm_proto::v1::TerminalSnapshot, needle: &[u8]) -> bool {
    contains(&snapshot.screen_ansi, needle) || contains(&snapshot.recent_history_ansi, needle)
}

fn snapshot_contains_domain(
    snapshot: &zterm_core::terminal::TerminalSnapshot,
    needle: &[u8],
) -> bool {
    contains(&snapshot.screen_ansi, needle) || contains(&snapshot.recent_history_ansi, needle)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
