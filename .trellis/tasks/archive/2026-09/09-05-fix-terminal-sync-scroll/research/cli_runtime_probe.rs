// Planning-only executable linked against the unchanged product libraries.
// The runner substitutes this existing fixture's absolute path before compiling.
#[path = "__DAEMON_HARNESS__"]
#[allow(dead_code)]
mod daemon_harness;

use clap::Parser;
use std::path::PathBuf;
use zterm_daemon::{lifecycle::DaemonLauncher, operations::LocalRuntime};
use zterm_platform::user_state::UserPaths;

fn main() {
    let arguments = std::env::args().collect::<Vec<_>>();
    if let Some(encoded) = arguments[1].strip_prefix("--probe-daemon=") {
        let paths = daemon_harness::decode_paths(encoded).expect("isolated paths");
        zterm_platform::local_unix::detach_current_process().expect("detach probe daemon");
        let identity = zterm_daemon::identity::DeviceIdentity::load(&paths).expect("identity");
        let cwd = paths.home().to_owned();
        let sessions = zterm_daemon::session::SessionService::with_spawner(
            identity.device_id(),
            zterm_core::ResourceLimits::default(),
            move |size, requested| {
                let cwd = requested.map(PathBuf::from).unwrap_or_else(|| cwd.clone());
                let command =
                    zterm_platform::pty::ExplicitPtyCommand::new("/bin/sh", &cwd).arg("-i");
                let pty = zterm_platform::pty::PtyHost::new()
                    .spawn(
                        command,
                        zterm_platform::pty::PtySize::new(size.rows, size.columns),
                    )
                    .map_err(|error| {
                        zterm_daemon::error::DaemonError::new(
                            zterm_core::DomainErrorKind::StoreUnavailable,
                            error.to_string(),
                        )
                    })?;
                Ok((pty, cwd))
            },
        );
        zterm_daemon::lifecycle::run_local_only_daemon_with_sessions_for_test(&paths, sessions)
            .expect("probe daemon");
        return;
    }
    let runtime = if arguments[1] == "--existing-state" {
        assert_eq!(arguments[2], "session");
        assert!(matches!(arguments[3].as_str(), "new" | "attach" | "close"));
        assert!(arguments[5].starts_with("zterm-causal-"));
        LocalRuntime::for_test(
            zterm_daemon::lifecycle::production_user_paths().expect("existing user paths"),
            DaemonLauncher::for_test("/does/not/exist".into(), "--must-not-start".into()),
        )
    } else {
        let root = PathBuf::from(&arguments[1]);
        let home = root.join("home");
        std::fs::create_dir_all(&home).expect("isolated home");
        let paths = UserPaths::for_test(
            nix::unistd::Uid::effective().as_raw(),
            home.clone(),
            home.join(".zterm"),
            root.join("run"),
        );
        let daemon_argument = format!("--probe-daemon={}", daemon_harness::encode_paths(&paths));
        LocalRuntime::for_test(
            paths,
            DaemonLauncher::for_test(
                std::env::current_exe().expect("probe executable"),
                daemon_argument,
            ),
        )
    };
    let cli = zterm_cli::Cli::try_parse_from(
        std::iter::once("zterm").chain(arguments[2..].iter().map(String::as_str)),
    )
    .expect("probe CLI arguments");
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = tokio.block_on(async {
        match zterm_cli::execute(cli, &runtime, zterm_cli::InteractionMode::NonInteractive).await? {
            zterm_cli::CommandOutcome::Text(text) => print!("{text}"),
            zterm_cli::CommandOutcome::Terminal(request) => {
                zterm_cli::run_terminal(request, &runtime).await?;
            }
            _ => panic!("unexpected probe outcome"),
        }
        Ok::<(), zterm_cli::CliError>(())
    });
    if let Err(error) = result {
        eprintln!("PROBE_CLI_ERROR={error}");
        std::process::exit(1);
    }
}
