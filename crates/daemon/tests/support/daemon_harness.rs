//! Multi-process daemon test-harness path encoding and child entry.

use std::ffi::OsString;
use std::path::PathBuf;

use zterm_platform::user_state::UserPaths;

const CHILD_PREFIX: &str = "--zterm-test-daemon=";

/// Encodes task-private paths into the one detached child harness argument.
pub fn child_argument(paths: &UserPaths) -> String {
    format!(
        "{CHILD_PREFIX}{}:{}:{}:{}",
        paths.uid(),
        encode_path(paths.home()),
        encode_path(paths.state_root()),
        encode_path(paths.runtime_dir())
    )
}

/// Runs the daemon child when the custom harness executable received its child argument.
pub fn run_child_if_requested() -> bool {
    let Some(argument) = std::env::args().nth(1) else {
        return false;
    };
    let Some(encoded) = argument.strip_prefix(CHILD_PREFIX) else {
        return false;
    };
    let paths = match decode_paths(encoded) {
        Ok(paths) => paths,
        Err(detail) => {
            eprintln!("invalid daemon harness paths: {detail}");
            std::process::exit(2);
        }
    };
    if let Err(error) = zterm_platform::local_unix::detach_current_process()
        .map_err(|error| error.to_string())
        .and_then(|()| {
            zterm_daemon::lifecycle::run_daemon(&paths).map_err(|error| error.to_string())
        })
    {
        eprintln!("daemon harness failed: {error}");
        std::process::exit(1);
    }
    true
}

fn decode_paths(encoded: &str) -> Result<UserPaths, String> {
    let fields = encoded.split(':').collect::<Vec<_>>();
    if fields.len() != 4 {
        return Err("expected uid and three hex paths".to_owned());
    }
    let uid = fields[0]
        .parse::<u32>()
        .map_err(|error| format!("invalid UID: {error}"))?;
    let home = decode_path(fields[1])?;
    let state = decode_path(fields[2])?;
    let runtime = decode_path(fields[3])?;
    Ok(UserPaths::for_test(uid, home, state, runtime))
}

#[cfg(unix)]
fn encode_path(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let mut encoded = String::with_capacity(path.as_os_str().as_bytes().len() * 2);
    for byte in path.as_os_str().as_bytes() {
        use std::fmt::Write;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(not(unix))]
fn encode_path(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(unix)]
fn decode_path(encoded: &str) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStringExt;

    if !encoded.len().is_multiple_of(2) {
        return Err("hex path has an odd length".to_owned());
    }
    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    let (pairs, remainder) = encoded.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return Err("hex path has an odd length".to_owned());
    }
    for pair in pairs {
        let pair = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
        bytes.push(u8::from_str_radix(pair, 16).map_err(|error| error.to_string())?);
    }
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(not(unix))]
fn decode_path(encoded: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(OsString::from(encoded)))
}
