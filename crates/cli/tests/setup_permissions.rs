//! First-setup validation and no-mutation failure acceptance tests.

#[path = "../../daemon/tests/support/state_fixture.rs"]
mod state_fixture;

use clap::Parser;
use zterm_cli::{Cli, CliError, InteractionMode, execute};
use zterm_daemon::lifecycle::DaemonLauncher;
use zterm_daemon::operations::LocalRuntime;

use state_fixture::TestState;

#[tokio::test]
async fn noninteractive_missing_values_fail_before_state_or_identity_creation() {
    let state = TestState::new();
    let runtime = LocalRuntime::for_test(
        state.paths.clone(),
        DaemonLauncher::for_test("/does/not/exist".into(), "--unused".to_owned()),
    );
    let command = Cli::try_parse_from(["zterm", "setup"]).expect("setup parses");
    let error = execute(command, &runtime, InteractionMode::NonInteractive)
        .await
        .expect_err("missing setup values fail");
    assert!(matches!(error, CliError::Usage(_)));
    assert!(!state.paths.state_root().exists());
    assert!(!state.paths.identity().exists());
    assert!(!state.paths.database().exists());

    let missing_relay = Cli::try_parse_from([
        "zterm",
        "setup",
        "--name",
        "host",
        "--profile",
        "self-hosted",
    ])
    .expect("partial setup parses");
    assert!(matches!(
        execute(missing_relay, &runtime, InteractionMode::NonInteractive).await,
        Err(CliError::Usage(_))
    ));
    assert!(!state.paths.state_root().exists());
}
