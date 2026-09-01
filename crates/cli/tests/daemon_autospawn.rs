//! Setup/restart autospawn and CLI projection acceptance harness.

#[cfg(unix)]
#[path = "../../daemon/tests/support/daemon_harness.rs"]
mod daemon_harness;
#[cfg(unix)]
#[path = "../../daemon/tests/support/state_fixture.rs"]
mod state_fixture;

#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::mpsc;

#[cfg(unix)]
use clap::Parser;
#[cfg(unix)]
use nix::sys::signal::{Signal, kill};
#[cfg(unix)]
use nix::unistd::Pid;
#[cfg(unix)]
use zterm_cli::{Cli, CommandOutcome, InteractionMode, execute, run_terminal};
#[cfg(unix)]
use zterm_core::{Revision, terminal::TerminalSize};
#[cfg(unix)]
use zterm_daemon::lifecycle::DaemonLauncher;
#[cfg(unix)]
use zterm_daemon::operations::LocalRuntime;

#[cfg(unix)]
use state_fixture::TestState;

#[cfg(unix)]
const TERMINAL_CHILD_PREFIX: &str = "--zterm-test-terminal=";
#[cfg(unix)]
const TERMINAL_ENTER_BYTES: &[u8] = b"\x1b[?1049h\x1b[?25l";
#[cfg(unix)]
const TERMINAL_RESTORE_BYTES: &[u8] = b"\x1b[?2004l\x1b[?1l\x1b>\x1b[0m\x1b[?25h\x1b[?1049l";
#[cfg(unix)]
const TERMINAL_CONNECT_MARKER: &[u8] = b"\xe7\x95\x8c";
#[cfg(unix)]
const TERMINAL_BARE_MARKER: &[u8] = b"\xe9\x9b\xaa";
#[cfg(unix)]
const TERMINAL_SHELL_READY_MARKER: &[u8] = b"ZTERM_LOCAL_UI_SHELL_READY";
#[cfg(unix)]
const TERMINAL_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(not(unix))]
fn main() {
    println!("DAEMON_AUTOSPAWN_GATE=SKIPPED_NON_UNIX");
}

#[cfg(unix)]
fn main() {
    if run_terminal_child_if_requested() {
        return;
    }
    if daemon_harness::run_child_if_requested() {
        return;
    }
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    let state = TestState::new();
    let executable = std::env::current_exe().expect("harness executable");
    let launcher = DaemonLauncher::for_test(
        executable,
        daemon_harness::terminal_child_argument(&state.paths),
    );
    let runtime = LocalRuntime::for_test(state.paths.clone(), launcher);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tokio.block_on(cli_autospawn(&state, runtime.clone()));
    }));
    if let Err(payload) = result {
        tokio.block_on(cleanup_failed_daemon(&runtime, &state.paths));
        std::panic::resume_unwind(payload);
    }
}

#[cfg(unix)]
fn run_terminal_child_if_requested() -> bool {
    let Some(argument) = std::env::args().nth(1) else {
        return false;
    };
    let Some(encoded) = argument.strip_prefix(TERMINAL_CHILD_PREFIX) else {
        return false;
    };
    let (mode, encoded_paths) = encoded
        .split_once(':')
        .expect("terminal child mode and task-private paths");
    let paths = daemon_harness::decode_paths(encoded_paths).expect("terminal child paths");
    let runtime = LocalRuntime::for_test(
        paths,
        DaemonLauncher::for_test("/does/not/exist".into(), "--must-not-run".to_owned()),
    );
    let arguments = match mode {
        "connect" => vec!["zterm", "connect", "local"],
        "bare-signal" => vec!["zterm"],
        "non-tty-connect" => vec!["zterm", "connect", "local"],
        _ => panic!("unknown terminal child mode"),
    };
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("terminal child runtime");
    let result = tokio.block_on(async {
        let outcome = execute(
            Cli::try_parse_from(arguments).expect("terminal child command"),
            &runtime,
            InteractionMode::NonInteractive,
        )
        .await?;
        let CommandOutcome::Terminal(request) = outcome else {
            panic!("terminal child command did not defer: {outcome:?}");
        };
        run_terminal(request, &runtime).await
    });
    if mode == "non-tty-connect" {
        if matches!(result, Err(zterm_cli::CliError::Usage(ref detail))
            if detail.contains("both stdin and stdout"))
        {
            return true;
        }
        eprintln!("non-TTY terminal child crossed preflight: {result:?}");
        std::process::exit(1);
    } else if mode == "bare-signal" {
        if matches!(result, Err(zterm_cli::CliError::Daemon(ref error))
            if error.kind() == zterm_core::DomainErrorKind::Cancelled)
        {
            return true;
        }
        eprintln!("signalled terminal child did not report cancellation");
        std::process::exit(1);
    } else if let Err(error) = result {
        eprintln!("local terminal child failed after restoration: {error:?}");
        let status = match &error {
            zterm_cli::CliError::Daemon(error) => match error.kind() {
                zterm_core::DomainErrorKind::DeadlineExceeded => 71,
                zterm_core::DomainErrorKind::Cancelled => 72,
                zterm_core::DomainErrorKind::LeaseLost => 73,
                _ => 74,
            },
            zterm_cli::CliError::TerminalDriverFailure => 75,
            zterm_cli::CliError::Io(_) => 76,
            zterm_cli::CliError::Usage(_)
            | zterm_cli::CliError::Serialization(_)
            | zterm_cli::CliError::CreatedSessionAttach { .. } => 77,
        };
        std::process::exit(status);
    }
    true
}

#[cfg(unix)]
async fn cli_autospawn(state: &TestState, runtime: LocalRuntime) {
    let first = run(
        &runtime,
        [
            "zterm",
            "setup",
            "--name",
            "cli-host",
            "--profile",
            "official-n0",
        ],
    )
    .await;
    assert!(first.contains("Configured cli-host"));
    assert!(state.paths.socket().exists(), "setup starts daemon");
    let key = std::fs::read(state.paths.identity()).expect("identity bytes");
    assert_eq!(key.len(), 32);
    assert_eq!(
        std::fs::metadata(state.paths.identity())
            .expect("identity metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(state.paths.state_root())
            .expect("state root metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    let repeated = run(&runtime, ["zterm", "setup"]).await;
    assert_eq!(first, repeated);
    assert_eq!(
        std::fs::read(state.paths.identity()).expect("stable identity"),
        key
    );
    let human = run(&runtime, ["zterm", "status"]).await;
    assert!(human.contains("State: running"));
    assert!(human.contains("Network: disabled"));
    assert!(human.contains("Endpoint bound: false"));
    assert!(human.contains("Address publish: disabled"));
    assert!(human.contains("Address lookup: disabled"));
    assert!(human.contains("Paths: direct=0, relay=0"));
    let json = run(&runtime, ["zterm", "status", "--json"]).await;
    let json: serde_json::Value = serde_json::from_str(&json).expect("running JSON");
    assert_eq!(json["state"], "running");
    assert_eq!(json["device_name"], "cli-host");
    assert_eq!(json["active_session_count"], 0);
    assert_eq!(json["network_state"], "disabled");
    assert_eq!(json["endpoint_bound"], false);
    assert_eq!(json["network_bind_attempts"], 0);
    assert_eq!(json["address_publish_state"], "disabled");
    assert_eq!(json["address_lookup_state"], "disabled");
    assert_eq!(json["authenticated_connection_count"], 0);
    assert_eq!(json["primary_connection_count"], 0);
    assert_eq!(json["active_stream_count"], 0);
    assert_eq!(json["direct_path_count"], 0);
    assert_eq!(json["relay_path_count"], 0);
    assert_eq!(json["network_diagnostic"], serde_json::Value::Null);
    let doctor = run(&runtime, ["zterm", "doctor", "--json"]).await;
    let doctor: serde_json::Value = serde_json::from_str(&doctor).expect("running doctor JSON");
    assert_eq!(doctor["healthy"], true);
    let running_network = doctor["checks"]
        .as_array()
        .expect("running doctor checks")
        .iter()
        .find(|check| check["name"] == "network")
        .expect("running network check");
    assert_eq!(running_network["ok"], true);
    assert!(
        running_network["detail"]
            .as_str()
            .expect("running network detail")
            .contains("state=disabled")
    );

    run(&runtime, ["zterm", "daemon", "stop"]).await;
    wait_for_socket(&state.paths, false).await;
    let stopped = run(&runtime, ["zterm", "status", "--json"]).await;
    let stopped: serde_json::Value = serde_json::from_str(&stopped).expect("stopped JSON");
    assert_eq!(stopped["state"], "configured_stopped");
    assert_eq!(stopped["network_state"], "stopped");
    assert_eq!(stopped["endpoint_bound"], false);
    assert_eq!(stopped["address_publish_state"], "disabled");
    assert_eq!(stopped["address_lookup_state"], "disabled");
    assert!(!state.paths.socket().exists(), "status does not restart");
    let doctor = run(&runtime, ["zterm", "doctor", "--json"]).await;
    let doctor: serde_json::Value = serde_json::from_str(&doctor).expect("stopped doctor JSON");
    assert_eq!(doctor["healthy"], true);
    let stopped_network = doctor["checks"]
        .as_array()
        .expect("stopped doctor checks")
        .iter()
        .find(|check| check["name"] == "network")
        .expect("stopped network check");
    assert_eq!(stopped_network["ok"], true);
    assert!(
        stopped_network["detail"]
            .as_str()
            .expect("stopped network detail")
            .contains("not attempted")
    );
    assert!(!state.paths.socket().exists(), "doctor does not restart");

    run_non_tty_terminal_child(&state.paths);
    assert!(
        !state.paths.socket().exists(),
        "non-TTY connect must fail before daemon autospawn"
    );

    let log_bytes = std::fs::read(state.paths.daemon_log()).expect("daemon log bytes");
    assert!(
        !log_bytes
            .windows(key.len())
            .any(|window| window == key.as_slice()),
        "daemon log must not contain raw identity bytes"
    );
    let mut log =
        zterm_platform::user_state::open_append(state.paths.daemon_log(), state.paths.uid())
            .expect("managed log append");
    for index in 0..1_100 {
        writeln!(log, "bounded-tail-{index}").expect("log fixture line");
    }
    drop(log);
    let tail = runtime.log_tail(2_000).expect("bounded log tail");
    assert_eq!(tail.len(), 1_000);
    assert_eq!(tail.first().map(String::as_str), Some("bounded-tail-100"));
    assert_eq!(tail.last().map(String::as_str), Some("bounded-tail-1099"));
    let rendered_log = run(&runtime, ["zterm", "logs", "--lines", "1"]).await;
    assert!(rendered_log.ends_with("bounded-tail-1099\n"));

    let unconfirmed_reset = execute(
        Cli::try_parse_from(["zterm", "reset", "--identity"]).expect("unconfirmed reset parses"),
        &runtime,
        InteractionMode::NonInteractive,
    )
    .await
    .expect_err("noninteractive identity reset requires confirmation");
    assert!(unconfirmed_reset.to_string().contains("--yes"));
    assert!(
        !state.paths.socket().exists(),
        "reset preflight never autospawns"
    );
    assert_eq!(
        std::fs::read(state.paths.identity()).expect("unconfirmed identity remains"),
        key
    );

    let restarted = run(&runtime, ["zterm", "daemon", "restart"]).await;
    assert!(restarted.contains("Daemon ready"));
    assert!(state.paths.socket().exists(), "restart starts daemon");
    assert_eq!(
        std::fs::read(state.paths.identity()).expect("restart identity"),
        key
    );

    let connect_output = run_local_terminal_child(&runtime, &state.paths, "connect").await;
    assert!(contains_bytes(&connect_output, TERMINAL_RESTORE_BYTES));
    let ui_sessions = runtime
        .session_list("local")
        .await
        .expect("list local UI Session");
    assert_eq!(ui_sessions.len(), 1);
    let ui_main_id = ui_sessions[0].session_id;
    wait_for_detach(&runtime, "local", &ui_main_id.to_string()).await;

    let bare_output = run_local_terminal_child(&runtime, &state.paths, "bare-signal").await;
    assert!(contains_bytes(&bare_output, TERMINAL_RESTORE_BYTES));
    let bare_sessions = runtime
        .session_list("local")
        .await
        .expect("list bare UI Session");
    assert_eq!(bare_sessions.len(), 1);
    assert_eq!(bare_sessions[0].session_id, ui_main_id);
    wait_for_detach(&runtime, "local", &ui_main_id.to_string()).await;

    expect_terminal(&runtime, ["zterm", "connect", "local"]).await;
    let viewport = Some(TerminalSize::new(24, 80));
    let main = runtime
        .attach("local", None, true, false, viewport)
        .await
        .expect("connect local main after deferred TTY preflight");
    let main_id = main.session_id();
    let sessions = run(&runtime, ["zterm", "session", "list", "local", "--json"]).await;
    let sessions: serde_json::Value = serde_json::from_str(&sessions).expect("session JSON");
    assert_eq!(sessions.as_array().expect("session list").len(), 1);
    assert_eq!(sessions[0]["session_id"], main_id.to_string());
    assert_eq!(sessions[0]["name"], "main");
    assert!(sessions[0].get("working_directory").is_none());
    drop(main);
    wait_for_detach(&runtime, "local", &main_id.to_string()).await;

    expect_terminal(&runtime, ["zterm"]).await;
    let bare = runtime
        .attach("local", None, true, false, viewport)
        .await
        .expect("bare local main after deferred TTY preflight");
    assert_eq!(
        bare.session_id(),
        main_id,
        "bare zterm reuses connect local main"
    );
    drop(bare);
    wait_for_detach(&runtime, "local", &main_id.to_string()).await;

    expect_terminal(&runtime, ["zterm", "session", "new", "local", "build"]).await;
    let created = runtime
        .session_create_for_attach("local", "build", None, viewport)
        .await
        .expect("create build after deferred TTY preflight");
    let build = runtime
        .attach_created(&created, viewport)
        .await
        .expect("attach exact created build Session");
    let build_id = build.session_id();
    expect_terminal(&runtime, ["zterm", "session", "attach", "local", "build"]).await;
    let occupied = runtime
        .attach("local", Some("build"), false, false, viewport)
        .await
        .expect_err("ordinary attach never steals an existing controller");
    assert!(occupied.to_string().contains("session_occupied"));
    expect_terminal(
        &runtime,
        ["zterm", "session", "attach", "local", "build", "--takeover"],
    )
    .await;
    let takeover = runtime
        .attach("local", Some("build"), false, true, viewport)
        .await
        .expect("takeover prepares after deferred TTY preflight");
    assert_eq!(takeover.session_id(), build_id);
    drop(takeover);

    let build_id_text = build_id.to_string();
    let renamed = run(
        &runtime,
        [
            "zterm",
            "session",
            "rename",
            "local",
            &build_id_text,
            "review",
        ],
    )
    .await;
    assert!(renamed.contains(&build_id.to_string()));
    assert!(renamed.contains("review"));
    let unconfirmed_close = execute(
        Cli::try_parse_from(["zterm", "session", "close", "local", "review"])
            .expect("unconfirmed close parses"),
        &runtime,
        InteractionMode::NonInteractive,
    )
    .await
    .expect_err("noninteractive Session close requires confirmation");
    assert!(unconfirmed_close.to_string().contains("--yes"));
    assert_eq!(
        runtime
            .session_close_preflight("local", "review")
            .await
            .expect("unconfirmed Session remains")
            .summary()
            .session_id,
        build_id
    );
    let closed = run(
        &runtime,
        ["zterm", "session", "close", "local", "review", "--yes"],
    )
    .await;
    assert!(closed.contains(&build_id.to_string()));
    drop(build);

    let reset_without_force = execute(
        Cli::try_parse_from(["zterm", "reset", "--identity", "--yes"]).expect("reset parses"),
        &runtime,
        InteractionMode::NonInteractive,
    )
    .await
    .expect_err("active main Session requires force");
    assert!(reset_without_force.to_string().contains("--force"));
    assert!(state.paths.identity().exists());
    assert!(state.paths.socket().exists());

    let reset = run(
        &runtime,
        ["zterm", "reset", "--identity", "--yes", "--force"],
    )
    .await;
    assert!(reset.contains("Managed identity state removed"));
    wait_for_socket(&state.paths, false).await;
    assert!(!state.paths.state_root().exists());
    assert!(!state.paths.identity().exists());
    assert!(!state.paths.config().exists());
    assert!(!state.paths.database().exists());

    let retry = run(
        &runtime,
        ["zterm", "reset", "--identity", "--yes", "--force"],
    )
    .await;
    assert!(retry.contains("already absent"));
    assert!(!state.paths.state_root().exists());

    let configured_again = run(
        &runtime,
        [
            "zterm",
            "setup",
            "--name",
            "cli-host-reset",
            "--profile",
            "official-n0",
        ],
    )
    .await;
    assert!(configured_again.contains("Configured cli-host-reset"));
    let replacement_key = std::fs::read(state.paths.identity()).expect("replacement identity");
    assert_ne!(replacement_key, key, "reset never performs automatic setup");
    run(&runtime, ["zterm", "daemon", "stop", "--force"]).await;
    wait_for_socket(&state.paths, false).await;
}

#[cfg(unix)]
async fn cleanup_failed_daemon(
    runtime: &LocalRuntime,
    paths: &zterm_platform::user_state::UserPaths,
) {
    let deadline = std::time::Instant::now() + TERMINAL_TEST_TIMEOUT;
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    let _ = tokio::time::timeout(remaining, runtime.stop(true)).await;
    while paths.socket().exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[cfg(unix)]
async fn run<const N: usize>(runtime: &LocalRuntime, arguments: [&str; N]) -> String {
    execute(
        Cli::try_parse_from(arguments).expect("command parses"),
        runtime,
        InteractionMode::NonInteractive,
    )
    .await
    .expect("command succeeds")
    .into_text()
    .expect("command returns ordinary text")
}

#[cfg(unix)]
async fn expect_terminal<const N: usize>(runtime: &LocalRuntime, arguments: [&str; N]) {
    match execute(
        Cli::try_parse_from(arguments).expect("command parses"),
        runtime,
        InteractionMode::NonInteractive,
    )
    .await
    .expect("terminal request defers")
    {
        CommandOutcome::Terminal(_) => {}
        outcome => panic!("expected deferred terminal request, got {outcome:?}"),
    }
}

#[cfg(unix)]
async fn wait_for_detach(runtime: &LocalRuntime, target: &str, selector: &str) {
    let expected = selector
        .parse::<zterm_core::SessionId>()
        .expect("detach probe uses canonical Session ID");
    let started = std::time::Instant::now();
    loop {
        let sessions = runtime
            .session_list(target)
            .await
            .expect("detach status list");
        if sessions
            .iter()
            .any(|session| session.session_id == expected && !session.has_controller)
        {
            return;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::task::yield_now().await;
    }
}

#[cfg(unix)]
async fn wait_for_socket(paths: &zterm_platform::user_state::UserPaths, expected: bool) {
    let started = std::time::Instant::now();
    while paths.socket().exists() != expected {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[cfg(unix)]
async fn run_local_terminal_child(
    runtime: &LocalRuntime,
    paths: &zterm_platform::user_state::UserPaths,
    mode: &str,
) -> Vec<u8> {
    use nix::pty::{Winsize, openpty};
    use nix::sys::termios::tcgetattr;

    let pty = openpty(
        Some(&Winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }),
        None,
    )
    .expect("open local terminal PTY");
    let master = File::from(pty.master);
    let mut master_writer = master.try_clone().expect("terminal master writer");
    let slave = File::from(pty.slave);
    let probe = slave.try_clone().expect("terminal termios probe");
    let original = tcgetattr(&probe).expect("terminal original termios");
    let child_stdin = slave.try_clone().expect("terminal child stdin");
    let child_stdout = slave.try_clone().expect("terminal child stdout");
    let child_stderr = slave.try_clone().expect("terminal child stderr");
    let argument = format!(
        "{TERMINAL_CHILD_PREFIX}{mode}:{}",
        daemon_harness::encode_paths(paths)
    );
    let mut child = Command::new(std::env::current_exe().expect("terminal harness executable"))
        .arg(argument)
        .stdin(Stdio::from(child_stdin))
        .stdout(Stdio::from(child_stdout))
        .stderr(Stdio::from(child_stderr))
        .spawn()
        .expect("spawn local terminal child");
    drop(slave);

    let output_marker = match mode {
        "connect" => TERMINAL_CONNECT_MARKER,
        "bare-signal" => TERMINAL_BARE_MARKER,
        _ => panic!("unsupported terminal PTY fixture mode"),
    };

    let (entered_sender, entered_receiver) = mpsc::channel();
    let (ready_sender, ready_receiver) = mpsc::channel();
    let (output_sender, output_receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut master = master;
        let mut bytes = Vec::new();
        let mut entered = false;
        let mut ready = false;
        let mut output_seen = false;
        let mut buffer = [0_u8; 4096];
        loop {
            match master.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    bytes.extend_from_slice(&buffer[..read]);
                    if !entered && contains_bytes(&bytes, TERMINAL_ENTER_BYTES) {
                        entered = true;
                        let _ = entered_sender.send(());
                    }
                    if !ready && contains_bytes(&bytes, TERMINAL_SHELL_READY_MARKER) {
                        ready = true;
                        let _ = ready_sender.send(());
                    }
                    let second_output_end = bytes
                        .windows(output_marker.len())
                        .enumerate()
                        .filter_map(|(index, window)| {
                            (window == output_marker).then_some(index + output_marker.len())
                        })
                        .nth(1);
                    if !output_seen && second_output_end.is_some() {
                        output_seen = true;
                        let _ = output_sender.send(());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.raw_os_error() == Some(nix::errno::Errno::EIO as i32) => break,
                Err(error) => panic!("read local terminal PTY: {error}"),
            }
        }
        bytes
    });

    if entered_receiver
        .recv_timeout(TERMINAL_TEST_TIMEOUT)
        .is_err()
    {
        terminate_failed_child(
            &mut child,
            &format!("local terminal {mode} did not enter raw mode"),
        );
    }
    if ready_receiver.recv_timeout(TERMINAL_TEST_TIMEOUT).is_err() {
        terminate_failed_child(
            &mut child,
            &format!("local terminal {mode} fixture did not become ready"),
        );
    }
    let active_revision = wait_for_active_viewport(runtime, 24, 80).await;
    // Session state can become Active just before the renderer consumes the
    // matching event. Use an idempotent fixture-only probe until the real PTY
    // echoes it; product input is still sent only through `run_terminal`.
    let probe_deadline = std::time::Instant::now() + TERMINAL_TEST_TIMEOUT;
    let output_seen = loop {
        master_writer
            .write_all(output_marker)
            .and_then(|()| master_writer.write_all(b"\r"))
            .expect("write deterministic interactive probe");
        if output_receiver
            .recv_timeout(std::time::Duration::from_millis(50))
            .is_ok()
        {
            break true;
        }
        if std::time::Instant::now() >= probe_deadline {
            break false;
        }
    };
    if !output_seen {
        let observed_revision = runtime
            .session_list("local")
            .await
            .ok()
            .and_then(|sessions| sessions.into_iter().find(|session| session.has_controller))
            .map(|session| session.revision);
        let child_state = match child.try_wait().expect("poll failed terminal child") {
            Some(status) => format!("exited_{:?}", status.code()),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                "running_until_fixture_cleanup".to_owned()
            }
        };
        drop(probe);
        drop(master_writer);
        let bytes = reader.join().unwrap_or_default();
        let marker_count = bytes
            .windows(output_marker.len())
            .filter(|window| *window == output_marker)
            .count();
        panic!(
            "local terminal {mode} did not render input/output; child_state={child_state}, child_error_kind={}, captured_bytes={}, marker_count={marker_count}, active_revision={}, observed_revision={observed_revision:?}",
            terminal_child_error_kind(&bytes),
            bytes.len(),
            active_revision.get(),
        );
    }
    if mode == "bare-signal" {
        set_terminal_child_size(&master_writer, 1, 5);
        kill(Pid::from_raw(child.id() as i32), Signal::SIGWINCH)
            .expect("notify narrow one-row viewport change");
        let narrow_revision = wait_for_active_viewport(runtime, 1, 5).await;
        assert!(
            narrow_revision.get() > active_revision.get(),
            "one-row SIGWINCH must advance the authoritative terminal revision"
        );

        for (rows, columns) in [(2, 37), (40, 120), (3, 9), (26, 82)] {
            set_terminal_child_size(&master_writer, rows, columns);
            kill(Pid::from_raw(child.id() as i32), Signal::SIGWINCH)
                .expect("notify rapid viewport change");
        }
        let resized_revision = wait_for_active_viewport(runtime, 26, 82).await;
        assert!(
            resized_revision.get() > narrow_revision.get(),
            "rapid SIGWINCH coalescing must publish the final authoritative viewport"
        );
        kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM)
            .expect("cancel resized terminal child");
    } else {
        master_writer
            .write_all(b"\x1d.")
            .expect("write local detach prefix");
    }
    let status = wait_for_terminal_child(&mut child, mode);
    assert_terminal_attributes_restored(
        tcgetattr(&probe).expect("terminal restored termios"),
        original,
    );
    drop(probe);
    drop(master_writer);
    let bytes = reader.join().expect("local terminal PTY reader");
    assert!(
        status.success(),
        "local terminal {mode} child failed: {status}"
    );
    let cleanup = find_bytes(&bytes, TERMINAL_RESTORE_BYTES).expect("terminal cleanup bytes");
    let output = find_bytes(&bytes, output_marker).expect("terminal output marker");
    assert!(
        output < cleanup,
        "rendered output must precede terminal cleanup"
    );
    bytes
}

#[cfg(unix)]
fn set_terminal_child_size(output: &File, rows: u16, columns: u16) {
    rustix::termios::tcsetwinsize(
        output,
        rustix::termios::Winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .expect("set terminal child viewport");
}

#[cfg(unix)]
async fn wait_for_active_viewport(runtime: &LocalRuntime, rows: u16, columns: u16) -> Revision {
    let deadline = std::time::Instant::now() + TERMINAL_TEST_TIMEOUT;
    loop {
        let active = runtime
            .session_list("local")
            .await
            .expect("observe terminal active barrier")
            .into_iter()
            .find(|session| {
                session.has_controller
                    && session.viewport.rows == rows
                    && session.viewport.columns == columns
            })
            .map(|session| session.revision);
        if let Some(revision) = active {
            return revision;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "terminal UI did not publish its active viewport"
        );
        tokio::task::yield_now().await;
    }
}

#[cfg(unix)]
fn run_non_tty_terminal_child(paths: &zterm_platform::user_state::UserPaths) {
    let argument = format!(
        "{TERMINAL_CHILD_PREFIX}non-tty-connect:{}",
        daemon_harness::encode_paths(paths)
    );
    let mut child = Command::new(std::env::current_exe().expect("terminal harness executable"))
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn non-TTY terminal child");
    let status = wait_for_terminal_child(&mut child, "non-tty-connect");
    assert!(
        status.success(),
        "non-TTY child must stop at terminal preflight"
    );
}

#[cfg(unix)]
fn wait_for_terminal_child(
    child: &mut std::process::Child,
    mode: &str,
) -> std::process::ExitStatus {
    let deadline = std::time::Instant::now() + TERMINAL_TEST_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().expect("poll local terminal child") {
            return status;
        }
        if std::time::Instant::now() >= deadline {
            terminate_failed_child(
                child,
                &format!("local terminal {mode} child did not exit after detach"),
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn terminate_failed_child(child: &mut std::process::Child, detail: &str) -> ! {
    let _ = child.kill();
    let _ = child.wait();
    panic!("{detail}");
}

#[cfg(unix)]
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    find_bytes(haystack, needle).is_some()
}

#[cfg(unix)]
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(unix)]
fn terminal_child_error_kind(bytes: &[u8]) -> &'static str {
    [
        (
            b"error_kind: DeadlineExceeded".as_slice(),
            "deadline_exceeded",
        ),
        (b"error_kind: Cancelled".as_slice(), "cancelled"),
        (b"error_kind: LeaseLost".as_slice(), "lease_lost"),
        (b"error_kind:".as_slice(), "other_daemon"),
        (b"TerminalDriverFailure".as_slice(), "terminal_driver"),
        (b"Io {".as_slice(), "io"),
        (b"Usage {".as_slice(), "usage"),
    ]
    .into_iter()
    .find_map(|(pattern, kind)| contains_bytes(bytes, pattern).then_some(kind))
    .unwrap_or("none")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn assert_terminal_attributes_restored(
    actual: nix::sys::termios::Termios,
    expected: nix::sys::termios::Termios,
) {
    assert_eq!(actual, expected);
}

#[cfg(target_os = "macos")]
fn assert_terminal_attributes_restored(
    actual: nix::sys::termios::Termios,
    expected: nix::sys::termios::Termios,
) {
    use nix::sys::termios::{LocalFlags, cfgetispeed, cfgetospeed};

    assert_eq!(actual.input_flags, expected.input_flags);
    assert_eq!(actual.output_flags, expected.output_flags);
    assert_eq!(actual.control_flags, expected.control_flags);
    assert_eq!(
        actual.local_flags - LocalFlags::PENDIN,
        expected.local_flags - LocalFlags::PENDIN
    );
    assert_eq!(actual.control_chars, expected.control_chars);
    assert_eq!(cfgetispeed(&actual), cfgetispeed(&expected));
    assert_eq!(cfgetospeed(&actual), cfgetospeed(&expected));
}
