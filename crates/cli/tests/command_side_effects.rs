//! Inspection-command output and no-autospawn acceptance tests.

#[path = "../../daemon/tests/support/state_fixture.rs"]
mod state_fixture;

use clap::{CommandFactory, Parser, error::ErrorKind};
use zterm_cli::{Cli, InteractionMode, execute};
use zterm_daemon::lifecycle::DaemonLauncher;
use zterm_daemon::operations::LocalRuntime;

use state_fixture::TestState;

#[tokio::test]
async fn help_version_status_doctor_logs_and_stop_never_spawn() {
    let state = TestState::new();
    let runtime = LocalRuntime::for_test(
        state.paths.clone(),
        DaemonLauncher::for_test("/does/not/exist".into(), "--must-not-run".to_owned()),
    );

    let mut definition = Cli::command();
    let help = definition.render_long_help().to_string();
    assert!(help.contains("setup"));
    assert!(help.contains("daemon"));
    assert!(!help.contains("internal-daemon"));
    let version = Cli::try_parse_from(["zterm", "--version"]).expect_err("clap prints version");
    assert_eq!(version.kind(), ErrorKind::DisplayVersion);
    assert!(version.to_string().contains(env!("CARGO_PKG_VERSION")));
    let binary = env!("CARGO_BIN_EXE_zterm");
    assert!(
        std::process::Command::new(binary)
            .arg("--help")
            .status()
            .expect("help process")
            .success()
    );
    assert!(
        std::process::Command::new(binary)
            .arg("--version")
            .status()
            .expect("version process")
            .success()
    );
    assert_eq!(
        std::process::Command::new(binary)
            .arg("--definitely-invalid")
            .status()
            .expect("invalid process")
            .code(),
        Some(2)
    );

    let human = run(&runtime, ["zterm", "status"]).await;
    assert!(human.contains("State: not_configured"));
    assert!(human.contains("Active sessions: 0"));
    let json = run(&runtime, ["zterm", "status", "--json"]).await;
    let json: serde_json::Value = serde_json::from_str(&json).expect("status JSON");
    assert_eq!(json["state"], "not_configured");
    assert_eq!(json["active_session_count"], 0);

    let doctor = run(&runtime, ["zterm", "doctor", "--json"]).await;
    let doctor: serde_json::Value = serde_json::from_str(&doctor).expect("doctor JSON");
    assert_eq!(doctor["healthy"], false);
    assert_eq!(doctor["checks"][1]["name"], "autostart");
    let checks = doctor["checks"].as_array().expect("doctor checks");
    let state_paths = checks
        .iter()
        .find(|check| check["name"] == "state_paths")
        .expect("state path check");
    assert_eq!(state_paths["ok"], false);
    assert!(
        state_paths["detail"]
            .as_str()
            .expect("state path detail")
            .contains(state.paths.state_root().to_string_lossy().as_ref())
    );
    assert_eq!(
        checks
            .iter()
            .find(|check| check["name"] == "local_ipc")
            .expect("local IPC check")["ok"],
        true
    );
    let daemon_status = run(&runtime, ["zterm", "daemon", "status"]).await;
    assert!(daemon_status.contains("not_configured"));
    let stop = run(&runtime, ["zterm", "daemon", "stop"]).await;
    assert_eq!(stop, "Daemon already stopped.\n");
    let logs = run(&runtime, ["zterm", "logs", "--lines", "10"]).await;
    assert!(logs.contains("daemon.log"));

    assert!(!state.paths.state_root().exists());
    assert!(!state.paths.runtime_dir().exists());
    assert!(!state.paths.socket().exists());
}

async fn run<const N: usize>(runtime: &LocalRuntime, arguments: [&str; N]) -> String {
    execute(
        Cli::try_parse_from(arguments).expect("command parses"),
        runtime,
        InteractionMode::NonInteractive,
    )
    .await
    .expect("inspection succeeds")
}
