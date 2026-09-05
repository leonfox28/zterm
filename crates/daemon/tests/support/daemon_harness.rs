//! Multi-process daemon test-harness path encoding and child entry.

use std::ffi::OsString;
use std::path::PathBuf;

use zterm_platform::user_state::UserPaths;

const CHILD_PREFIX: &str = "--zterm-test-daemon=";
const TERMINAL_CHILD_PREFIX: &str = "--zterm-test-terminal-daemon=";

/// Encodes task-private paths into the one detached child harness argument.
#[allow(dead_code)]
pub fn child_argument(paths: &UserPaths) -> String {
    format!("{CHILD_PREFIX}{}", encode_paths(paths))
}

/// Hidden detached-child argument for a daemon with a deterministic PTY fixture.
#[allow(dead_code)]
pub fn terminal_child_argument(paths: &UserPaths) -> String {
    format!("{TERMINAL_CHILD_PREFIX}{}", encode_paths(paths))
}

/// Encodes task-private paths for another test-only child role.
pub fn encode_paths(paths: &UserPaths) -> String {
    format!(
        "{}:{}:{}:{}",
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
    let (encoded, deterministic_terminal) =
        if let Some(encoded) = argument.strip_prefix(CHILD_PREFIX) {
            (encoded, false)
        } else if let Some(encoded) = argument.strip_prefix(TERMINAL_CHILD_PREFIX) {
            (encoded, true)
        } else {
            return false;
        };
    let paths = match decode_paths(encoded) {
        Ok(paths) => paths,
        Err(detail) => {
            eprintln!("invalid daemon harness paths: {detail}");
            std::process::exit(2);
        }
    };
    let result = zterm_platform::local_unix::detach_current_process()
        .map_err(|error| error.to_string())
        .and_then(|()| {
            if deterministic_terminal {
                let sessions = terminal_fixture_sessions(&paths)?;
                zterm_daemon::lifecycle::run_local_only_daemon_with_sessions_for_test(
                    &paths, sessions,
                )
                .map_err(|error| error.to_string())
            } else {
                zterm_daemon::lifecycle::run_local_only_daemon_for_test(&paths)
                    .map_err(|error| error.to_string())
            }
        });
    if let Err(error) = result {
        eprintln!("daemon harness failed: {error}");
        std::process::exit(1);
    }
    true
}

fn terminal_fixture_sessions(
    paths: &UserPaths,
) -> Result<zterm_daemon::session::SessionService, String> {
    let own_device_id = zterm_daemon::identity::DeviceIdentity::load(paths)
        .map_err(|error| error.to_string())?
        .device_id();
    let shell = [PathBuf::from("/bin/sh"), PathBuf::from("/usr/bin/sh")]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "deterministic terminal fixture requires a POSIX shell".to_owned())?;
    let default_working_directory = paths.home().to_path_buf();
    Ok(zterm_daemon::session::SessionService::with_spawner(
        own_device_id,
        zterm_core::ResourceLimits::default(),
        move |size, requested_working_directory| {
            let working_directory = requested_working_directory
                .map(PathBuf::from)
                .unwrap_or_else(|| default_working_directory.clone());
            let session = zterm_platform::pty::PtyHost::new()
                .spawn(
                    zterm_platform::pty::ExplicitPtyCommand::new(&shell, &working_directory)
                        .arg("-c")
                        .arg(
                            "printf 'ZTERM_LOCAL_UI_SHELL_READY\\r\\n'; while IFS= read -r line; do case \"$line\" in ZTERM_TEST_DECSET_1049) printf '\\033[?1049hZTERM_ALT_READY\\r\\n' ;; ZTERM_TEST_DECRST_1049) printf '\\033[?1049lZTERM_MAIN_READY\\r\\n' ;; *) printf '%s\\r\\n%s\\r\\n' \"$line\" \"$line\" ;; esac; done",
                        ),
                    zterm_platform::pty::PtySize::new(size.rows, size.columns),
                )
                .map_err(|_| {
                    zterm_daemon::error::DaemonError::new(
                        zterm_core::DomainErrorKind::StoreUnavailable,
                        "deterministic terminal fixture could not start",
                    )
                })?;
            Ok((session, working_directory))
        },
    ))
}

/// Decodes task-private paths carried by a test-only child role.
pub fn decode_paths(encoded: &str) -> Result<UserPaths, String> {
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
