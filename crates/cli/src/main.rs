//! zterm command-line executable entry.

use std::io::{self, Write};
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
        Ok(zterm_cli::CommandOutcome::Text(output)) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Ok(zterm_cli::CommandOutcome::PairTicket(output)) => {
            let result = {
                let stdout = io::stdout();
                let mut stdout = stdout.lock();
                write_pair_ticket(&mut stdout, output.as_bytes())
            };
            drop(output);
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("unable to write pair ticket: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Ok(zterm_cli::CommandOutcome::Terminal(request)) => {
            match tokio.block_on(zterm_cli::run_terminal(request, &runtime)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn write_pair_ticket(writer: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
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

        write_pair_ticket(&mut writer, b"opaque-ticket\n").expect("ticket stdout");

        assert_eq!(writer.bytes, b"opaque-ticket\n");
        assert_eq!(writer.flushes, 1);
    }
}
