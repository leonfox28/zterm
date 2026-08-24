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
    let public_commands = definition
        .get_subcommands()
        .map(clap::Command::get_name)
        .collect::<Vec<_>>();
    for available in [
        "setup", "status", "doctor", "pair", "device", "connect", "session", "daemon", "logs",
        "reset",
    ] {
        assert!(
            public_commands.contains(&available),
            "{available} must be exposed by the public CLI milestone"
        );
    }
    for forbidden in ["--state", "--identity-path", "--socket", "--ticket"] {
        assert!(!help.contains(forbidden));
    }
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
    let stop_help = std::process::Command::new(binary)
        .args(["daemon", "stop", "--help"])
        .output()
        .expect("daemon stop help process");
    assert!(stop_help.status.success());
    let stop_help = String::from_utf8(stop_help.stdout).expect("daemon stop help is UTF-8");
    assert!(stop_help.contains("end active Sessions and their PTYs"));
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
    assert_eq!(json["network_state"], serde_json::Value::Null);
    assert_eq!(json["address_publish_state"], serde_json::Value::Null);
    assert_eq!(json["address_lookup_state"], serde_json::Value::Null);
    assert_eq!(json["authenticated_connection_count"], 0);
    assert_eq!(json["direct_path_count"], 0);
    assert_eq!(json["relay_path_count"], 0);
    assert_eq!(json["network_diagnostic"], serde_json::Value::Null);

    let doctor = run(&runtime, ["zterm", "doctor", "--json"]).await;
    let doctor: serde_json::Value = serde_json::from_str(&doctor).expect("doctor JSON");
    assert_eq!(doctor["healthy"], false);
    assert_eq!(doctor["checks"][1]["name"], "autostart");
    let checks = doctor["checks"].as_array().expect("doctor checks");
    let network = checks
        .iter()
        .find(|check| check["name"] == "network")
        .expect("network check");
    assert_eq!(network["ok"], true);
    assert!(
        network["detail"]
            .as_str()
            .expect("network detail")
            .contains("not attempted")
    );
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
    assert!(logs.is_empty());

    let bare = run(&runtime, ["zterm"]).await;
    assert_eq!(bare, "zterm is not configured. Run `zterm setup` first.\n");
    let restart = execute(
        Cli::try_parse_from(["zterm", "daemon", "restart"]).expect("restart parses"),
        &runtime,
        InteractionMode::NonInteractive,
    )
    .await
    .expect_err("restart must validate setup before spawning");
    assert!(restart.to_string().contains("run `zterm setup`"));

    for arguments in [
        vec!["zterm", "pair", "create"],
        vec!["zterm", "device", "list"],
        vec!["zterm", "session", "list", "local"],
    ] {
        let error = execute(
            Cli::try_parse_from(arguments).expect("daemon-required command parses"),
            &runtime,
            InteractionMode::NonInteractive,
        )
        .await
        .expect_err("daemon-required command validates setup before spawning");
        assert!(error.to_string().contains("run `zterm setup`"));
    }
    let no_tty_ticket = execute(
        Cli::try_parse_from(["zterm", "pair", "accept"]).expect("pair accept parses"),
        &runtime,
        InteractionMode::NonInteractive,
    )
    .await
    .expect_err("pair accept validates setup before reading a ticket");
    assert!(no_tty_ticket.to_string().contains("run `zterm setup`"));

    assert!(
        Cli::try_parse_from(["zterm", "pair", "accept", "secret-ticket"]).is_err(),
        "pair tickets are never accepted from argv"
    );
    assert!(Cli::try_parse_from(["zterm", "reset"]).is_err());

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
    .into_text()
    .expect("inspection returns ordinary text")
}
