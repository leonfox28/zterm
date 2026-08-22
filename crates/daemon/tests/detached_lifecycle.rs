//! Detached launch, graceful restart, and crash-recovery acceptance harness.

#[cfg(unix)]
#[path = "support/daemon_harness.rs"]
mod daemon_harness;
#[cfg(unix)]
#[path = "support/state_fixture.rs"]
mod state_fixture;

#[cfg(unix)]
use zterm_core::DomainErrorKind;
#[cfg(unix)]
use zterm_daemon::bootstrap::{bootstrap, validate_committed_setup};
#[cfg(unix)]
use zterm_daemon::config::{ValidatedInfrastructure, validate_setup_input};
#[cfg(unix)]
use zterm_daemon::lifecycle::ensure_daemon_with;
#[cfg(unix)]
use zterm_daemon::local_ipc::LocalClient;

#[cfg(unix)]
use state_fixture::TestState;

#[cfg(not(unix))]
fn main() {
    println!("DETACHED_LIFECYCLE_GATE=SKIPPED_NON_UNIX");
}

#[cfg(unix)]
fn main() {
    if daemon_harness::run_child_if_requested() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(detached_lifecycle());
}

#[cfg(unix)]
async fn detached_lifecycle() {
    let state = TestState::new();
    let requested =
        validate_setup_input("detached", ValidatedInfrastructure::OfficialN0).expect("valid setup");
    let setup = bootstrap(&state.paths, &requested).expect("bootstrap");
    let identity_bytes = std::fs::read(state.paths.identity()).expect("identity snapshot");
    let executable = std::env::current_exe().expect("harness executable");
    let argument = daemon_harness::child_argument(&state.paths);

    ensure_daemon_with(&state.paths, &executable, &argument)
        .await
        .expect("detached readiness");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let client = LocalClient::new(state.paths.socket());
    assert_eq!(
        client.status().await.expect("survives launcher").device_id,
        setup.device_id
    );
    client.stop(false).await.expect("explicit stop");
    wait_for_socket_state(&state.paths, false).await;

    ensure_daemon_with(&state.paths, &executable, &argument)
        .await
        .expect("restart readiness");
    assert_eq!(
        LocalClient::new(state.paths.socket())
            .status()
            .await
            .expect("restart status")
            .device_id,
        setup.device_id
    );
    LocalClient::new(state.paths.socket())
        .stop(false)
        .await
        .expect("restart stop");
    wait_for_socket_state(&state.paths, false).await;

    let mut crashed =
        zterm_platform::local_unix::detached_command(&executable, &state.paths, &argument)
            .expect("crash harness command")
            .spawn()
            .expect("crash harness spawn");
    wait_for_socket_state(&state.paths, true).await;
    LocalClient::new(state.paths.socket())
        .readiness()
        .await
        .expect("crash harness ready");
    crashed.kill().expect("known child kill");
    crashed.wait().expect("known child reap");
    assert!(state.paths.socket().exists(), "crash leaves stale socket");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        LocalClient::new(state.paths.socket())
            .readiness()
            .await
            .expect_err("no automatic restart")
            .kind(),
        DomainErrorKind::DaemonStopped
    );

    ensure_daemon_with(&state.paths, &executable, &argument)
        .await
        .expect("on-demand crash recovery");
    let recovered = LocalClient::new(state.paths.socket())
        .status()
        .await
        .expect("recovered status");
    assert_eq!(recovered.device_id, setup.device_id);
    assert_eq!(recovered.active_session_count, 0);
    LocalClient::new(state.paths.socket())
        .stop(false)
        .await
        .expect("recovery stop");
    wait_for_socket_state(&state.paths, false).await;
    assert_eq!(
        std::fs::read(state.paths.identity()).expect("stable identity bytes"),
        identity_bytes
    );
    assert_eq!(
        validate_committed_setup(&state.paths)
            .expect("committed setup remains valid")
            .device_id,
        setup.device_id
    );
}

#[cfg(unix)]
async fn wait_for_socket_state(paths: &zterm_platform::user_state::UserPaths, expected: bool) {
    let started = std::time::Instant::now();
    while paths.socket().exists() != expected {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "socket did not reach expected state {expected}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
