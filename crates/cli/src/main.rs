//! zterm command-line executable entry.

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = zterm_cli::Cli::parse();
    if cli.internal_daemon() {
        return match zterm_daemon::lifecycle::run_internal_daemon() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }

    let runtime = match zterm_daemon::operations::LocalRuntime::current() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let tokio = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(tokio) => tokio,
        Err(error) => {
            eprintln!("unable to initialize zterm runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match tokio.block_on(zterm_cli::execute(
        cli,
        &runtime,
        zterm_cli::InteractionMode::detect(),
    )) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
