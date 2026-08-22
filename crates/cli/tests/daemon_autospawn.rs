//! Setup/restart autospawn and CLI projection acceptance harness.

#[cfg(unix)]
#[path = "../../daemon/tests/support/daemon_harness.rs"]
mod daemon_harness;
#[cfg(unix)]
#[path = "../../daemon/tests/support/state_fixture.rs"]
mod state_fixture;

#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
use clap::Parser;
#[cfg(unix)]
use zterm_cli::{Cli, InteractionMode, execute};
#[cfg(unix)]
use zterm_daemon::lifecycle::DaemonLauncher;
#[cfg(unix)]
use zterm_daemon::operations::LocalRuntime;

#[cfg(unix)]
use state_fixture::TestState;

#[cfg(not(unix))]
fn main() {
    println!("DAEMON_AUTOSPAWN_GATE=SKIPPED_NON_UNIX");
}

#[cfg(unix)]
fn main() {
    if daemon_harness::run_child_if_requested() {
        return;
    }
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    tokio.block_on(cli_autospawn());
}

#[cfg(unix)]
async fn cli_autospawn() {
    let state = TestState::new();
    let executable = std::env::current_exe().expect("harness executable");
    let launcher =
        DaemonLauncher::for_test(executable, daemon_harness::child_argument(&state.paths));
    let runtime = LocalRuntime::for_test(state.paths.clone(), launcher);

    let first = run(
        &runtime,
        [
            "zterm",
            "setup",
            "--name",
            "cli-host",
            "--profile",
            "official-n0",
        ],
    )
    .await;
    assert!(first.contains("Configured cli-host"));
    assert!(state.paths.socket().exists(), "setup starts daemon");
    let key = std::fs::read(state.paths.identity()).expect("identity bytes");
    assert_eq!(key.len(), 32);
    assert_eq!(
        std::fs::metadata(state.paths.identity())
            .expect("identity metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(state.paths.state_root())
            .expect("state root metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    let repeated = run(&runtime, ["zterm", "setup"]).await;
    assert_eq!(first, repeated);
    assert_eq!(
        std::fs::read(state.paths.identity()).expect("stable identity"),
        key
    );
    let human = run(&runtime, ["zterm", "status"]).await;
    assert!(human.contains("State: running"));
    let json = run(&runtime, ["zterm", "status", "--json"]).await;
    let json: serde_json::Value = serde_json::from_str(&json).expect("running JSON");
    assert_eq!(json["state"], "running");
    assert_eq!(json["device_name"], "cli-host");
    assert_eq!(json["active_session_count"], 0);
    let doctor = run(&runtime, ["zterm", "doctor", "--json"]).await;
    let doctor: serde_json::Value = serde_json::from_str(&doctor).expect("running doctor JSON");
    assert_eq!(doctor["healthy"], true);

    run(&runtime, ["zterm", "daemon", "stop"]).await;
    wait_for_socket(&state.paths, false).await;
    let stopped = run(&runtime, ["zterm", "status", "--json"]).await;
    let stopped: serde_json::Value = serde_json::from_str(&stopped).expect("stopped JSON");
    assert_eq!(stopped["state"], "configured_stopped");
    assert!(!state.paths.socket().exists(), "status does not restart");
    let doctor = run(&runtime, ["zterm", "doctor", "--json"]).await;
    let doctor: serde_json::Value = serde_json::from_str(&doctor).expect("stopped doctor JSON");
    assert_eq!(doctor["healthy"], true);

    let log_bytes = std::fs::read(state.paths.daemon_log()).expect("daemon log bytes");
    assert!(
        !log_bytes
            .windows(key.len())
            .any(|window| window == key.as_slice()),
        "daemon log must not contain raw identity bytes"
    );
    let mut log =
        zterm_platform::user_state::open_append(state.paths.daemon_log(), state.paths.uid())
            .expect("managed log append");
    for index in 0..1_100 {
        writeln!(log, "bounded-tail-{index}").expect("log fixture line");
    }
    drop(log);
    let tail = runtime.log_tail(2_000).expect("bounded log tail");
    assert_eq!(tail.len(), 1_000);
    assert_eq!(tail.first().map(String::as_str), Some("bounded-tail-100"));
    assert_eq!(tail.last().map(String::as_str), Some("bounded-tail-1099"));
    let rendered_log = run(&runtime, ["zterm", "logs", "--lines", "1"]).await;
    assert!(rendered_log.ends_with("bounded-tail-1099\n"));

    let restarted = run(&runtime, ["zterm", "daemon", "restart"]).await;
    assert!(restarted.contains("Daemon ready"));
    assert!(state.paths.socket().exists(), "restart starts daemon");
    assert_eq!(
        std::fs::read(state.paths.identity()).expect("restart identity"),
        key
    );
    run(&runtime, ["zterm", "daemon", "stop"]).await;
    wait_for_socket(&state.paths, false).await;
}

#[cfg(unix)]
async fn run<const N: usize>(runtime: &LocalRuntime, arguments: [&str; N]) -> String {
    execute(
        Cli::try_parse_from(arguments).expect("command parses"),
        runtime,
        InteractionMode::NonInteractive,
    )
    .await
    .expect("command succeeds")
}

#[cfg(unix)]
async fn wait_for_socket(paths: &zterm_platform::user_state::UserPaths, expected: bool) {
    let started = std::time::Instant::now();
    while paths.socket().exists() != expected {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
