//! Multi-process singleflight launch acceptance harness.

#[cfg(unix)]
#[path = "support/daemon_harness.rs"]
mod daemon_harness;
#[cfg(unix)]
#[path = "support/state_fixture.rs"]
mod state_fixture;

#[cfg(unix)]
use std::sync::Arc;

#[cfg(unix)]
use zterm_daemon::bootstrap::{bootstrap, validate_committed_setup};
#[cfg(unix)]
use zterm_daemon::config::{ValidatedInfrastructure, validate_setup_input};
#[cfg(unix)]
use zterm_daemon::lifecycle::ensure_daemon_with;
#[cfg(unix)]
use zterm_daemon::local_ipc::LocalClient;
#[cfg(unix)]
use zterm_platform::local_unix::DaemonLock;

#[cfg(unix)]
use state_fixture::TestState;

#[cfg(not(unix))]
fn main() {
    println!("SINGLE_INSTANCE_GATE=SKIPPED_NON_UNIX");
}

#[cfg(unix)]
fn main() {
    if daemon_harness::run_child_if_requested() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(singleflight_launch());
}

#[cfg(unix)]
async fn singleflight_launch() {
    let state = TestState::new();
    let requested = validate_setup_input("singleflight", ValidatedInfrastructure::OfficialN0)
        .expect("valid setup");
    let setup = bootstrap(&state.paths, &requested).expect("bootstrap");
    let executable = std::env::current_exe().expect("harness executable");
    let argument = Arc::new(daemon_harness::child_argument(&state.paths));
    let paths = Arc::new(state.paths.clone());
    let executable = Arc::new(executable);
    let mut launchers = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let paths = Arc::clone(&paths);
        let executable = Arc::clone(&executable);
        let argument = Arc::clone(&argument);
        launchers.spawn(async move {
            ensure_daemon_with(&paths, &executable, &argument)
                .await
                .expect("launcher readiness")
        });
    }
    let mut readiness = Vec::new();
    while let Some(result) = launchers.join_next().await {
        readiness.push(result.expect("launcher task"));
    }
    assert_eq!(readiness.len(), 12);
    assert!(readiness.windows(2).all(|pair| pair[0] == pair[1]));
    assert!(
        DaemonLock::try_acquire(&paths)
            .expect("lock probe")
            .is_none(),
        "one daemon retains the lifetime lock"
    );
    let client = LocalClient::new(paths.socket());
    let status = client.status().await.expect("running status");
    assert_eq!(status.device_id, setup.device_id);
    assert_eq!(status.active_session_count, 0);
    client.stop(false).await.expect("graceful stop");
    wait_for_stop(&paths).await;
    assert_eq!(
        validate_committed_setup(&paths)
            .expect("identity remains committed")
            .device_id,
        setup.device_id
    );
}

#[cfg(unix)]
async fn wait_for_stop(paths: &zterm_platform::user_state::UserPaths) {
    let started = std::time::Instant::now();
    loop {
        if !paths.socket().exists()
            && DaemonLock::try_acquire(paths)
                .expect("post-stop lock probe")
                .is_some()
        {
            return;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "daemon did not release socket/lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
