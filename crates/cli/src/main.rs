//! zterm command-line executable entry.

use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = zterm_cli::Cli::parse();
    if cli.internal_update() {
        #[cfg(unix)]
        return match zterm_daemon::update::run_internal_update() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
        #[cfg(not(unix))]
        return ExitCode::FAILURE;
    }
    if cli.internal_daemon() {
        return match zterm_daemon::lifecycle::run_internal_daemon() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if cli.internal_release_self_check() {
        return match zterm_daemon::distribution::self_check_json() {
            Ok(output) => {
                print!("{output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some((manifest, signature)) = cli.internal_release_verify() {
        return match zterm_daemon::distribution::verify_candidate_files(manifest, signature) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(destination) = cli.internal_release_install() {
        return match zterm_daemon::distribution::install_current_executable(destination) {
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
            eprintln!("Error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let tokio = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(tokio) => tokio,
        Err(error) => {
            eprintln!("Error: Unable to initialize zterm runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match tokio.block_on(zterm_cli::execute(
        cli,
        &runtime,
        zterm_cli::InteractionMode::detect(),
    )) {
        Ok(zterm_cli::CommandOutcome::Text(output)) => {
            match write_output(&mut io::stdout().lock(), output.as_bytes()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Error: Unable to write command output: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Ok(zterm_cli::CommandOutcome::PairTicket(output)) => {
            let result = {
                let stdout = io::stdout();
                let mut stdout = stdout.lock();
                write_output(&mut stdout, output.as_bytes())
            };
            drop(output);
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Error: Unable to write pair ticket: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Ok(zterm_cli::CommandOutcome::Terminal(request)) => {
            match tokio.block_on(zterm_cli::run_terminal(request, &runtime)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Error: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn write_output(writer: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    writer.write_all(bytes)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingWriter {
        bytes: Vec<u8>,
        flushes: usize,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn pair_ticket_stdout_is_exact_and_explicitly_flushed() {
        let mut writer = RecordingWriter::default();

        write_output(&mut writer, b"opaque-ticket\n").expect("ticket stdout");

        assert_eq!(writer.bytes, b"opaque-ticket\n");
        assert_eq!(writer.flushes, 1);
    }

    #[test]
    fn output_write_and_flush_failures_are_not_success() {
        struct FailingWriter {
            fail_write: bool,
        }
        impl Write for FailingWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.fail_write {
                    Err(io::ErrorKind::BrokenPipe.into())
                } else {
                    Ok(bytes.len())
                }
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
        }
        for fail_write in [true, false] {
            assert!(
                write_output(&mut FailingWriter { fail_write }, b"synthetic-ticket\n").is_err()
            );
        }
    }
}
