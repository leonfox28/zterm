//! Explicit disposable host/CLI fixture for emulator and outer-terminal acceptance.
//! `init <new-root>`, `start <root>`, `pair <root>`, or `cli <root> <zterm args...>`.
//! Every path comes from the new root; the user's daemon is never discovered.
#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clap::Parser;
    use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf};
    use zterm_daemon::{
        bootstrap::bootstrap,
        client::LocalPairingClient,
        config::{ValidatedInfrastructure, validate_setup_input},
        lifecycle::{DaemonLauncher, run_daemon},
        operations::LocalRuntime,
    };
    use zterm_platform::user_state::UserPaths;
    let mut args = std::env::args().skip(1);
    let action = args.next().ok_or("fixture action required")?;
    let root = PathBuf::from(args.next().ok_or("explicit disposable root required")?);
    if action == "init" {
        fs::create_dir(&root)?;
    }
    let root = root.canonicalize()?;
    let marker = root.join(".presentation-fixture");
    if action == "init" {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&marker)?;
        fs::create_dir(root.join("home"))?;
    } else if !marker.is_file() {
        return Err("not a presentation fixture root".into());
    }
    let paths = UserPaths::for_test(
        nix::unistd::Uid::effective().as_raw(),
        root.join("home"),
        root.join("state"),
        root.join("run"),
    );
    if action == "init" {
        let config =
            validate_setup_input("presentation-fixture", ValidatedInfrastructure::OfficialN0)?;
        bootstrap(&paths, &config)?;
        return Ok(());
    }
    if action == "start" {
        run_daemon(&paths)?;
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        if action == "pair" {
            let ticket = LocalPairingClient::new(paths.socket()).create(300).await?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(root.join("pairing-fixture.txt"))?;
            file.write_all(ticket.expose().as_bytes())?;
        } else if action == "cli" {
            let local = LocalRuntime::for_test(
                paths,
                DaemonLauncher::for_test("/must-not-launch".into(), "--unused".into()),
            );
            let cli =
                zterm_cli::Cli::try_parse_from(std::iter::once("zterm".to_owned()).chain(args))?;
            match zterm_cli::execute(cli, &local, zterm_cli::InteractionMode::detect()).await? {
                zterm_cli::CommandOutcome::Text(text) => print!("{text}"),
                zterm_cli::CommandOutcome::Terminal(request) => {
                    zterm_cli::run_terminal(request, &local).await?;
                }
                _ => return Err("use the fixture pair action for bearer tickets".into()),
            }
        } else {
            return Err("unknown fixture action".into());
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

#[cfg(not(unix))]
fn main() {
    eprintln!("presentation fixture requires a Unix PTY host");
}
