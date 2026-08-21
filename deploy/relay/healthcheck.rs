//! One-shot HTTP health probe for the shell-free relay runtime image.

use std::{
    env,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    process::ExitCode,
    str,
    time::Duration,
};

const IO_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let address = args
        .next()
        .ok_or_else(|| "missing ADDRESS argument".to_owned())?
        .parse::<SocketAddr>()
        .map_err(|error| format!("invalid ADDRESS: {error}"))?;
    let path = args
        .next()
        .ok_or_else(|| "missing PATH argument".to_owned())?;
    if !path.starts_with('/') {
        return Err("PATH must start with '/'".to_owned());
    }
    let expected_status = args
        .next()
        .ok_or_else(|| "missing EXPECTED_STATUS argument".to_owned())?
        .parse::<u16>()
        .map_err(|error| format!("invalid EXPECTED_STATUS: {error}"))?;
    let expected_body = args.next();
    if args.next().is_some() {
        return Err("too many arguments".to_owned());
    }

    let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT)
        .map_err(|error| format!("failed to connect to {address}: {error}"))?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("failed to set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("failed to set write timeout: {error}"))?;

    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("failed to write request: {error}"))?;

    let mut response = Vec::new();
    stream
        .take(MAX_RESPONSE_BYTES)
        .read_to_end(&mut response)
        .map_err(|error| format!("failed to read response: {error}"))?;

    let first_line_end = response
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| "response did not contain an HTTP status line".to_owned())?;
    let status_line = str::from_utf8(&response[..first_line_end])
        .map_err(|error| format!("HTTP status line was not UTF-8: {error}"))?
        .trim_end_matches('\r');
    let mut status_parts = status_line.split_whitespace();
    let protocol = status_parts
        .next()
        .ok_or_else(|| "HTTP status line was empty".to_owned())?;
    if protocol != "HTTP/1.1" && protocol != "HTTP/1.0" {
        return Err(format!(
            "unexpected HTTP protocol in status line: {status_line}"
        ));
    }
    let actual_status = status_parts
        .next()
        .ok_or_else(|| format!("missing status code in: {status_line}"))?
        .parse::<u16>()
        .map_err(|error| format!("invalid HTTP status code: {error}"))?;
    if actual_status != expected_status {
        return Err(format!(
            "expected HTTP {expected_status}, received {actual_status}"
        ));
    }

    if let Some(expected_body) = expected_body.filter(|value| !value.is_empty())
        && !response
            .windows(expected_body.len())
            .any(|window| window == expected_body.as_bytes())
    {
        return Err(format!(
            "HTTP response did not contain required marker: {expected_body}"
        ));
    }

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("relay health check failed: {error}");
            ExitCode::FAILURE
        }
    }
}
