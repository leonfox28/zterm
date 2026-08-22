//! Linux cross-UID peer-credential acceptance harness.

#[cfg(unix)]
#[path = "support/state_fixture.rs"]
mod state_fixture;

#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::sync::Arc;

#[cfg(unix)]
use zterm_daemon::bootstrap::bootstrap;
#[cfg(unix)]
use zterm_daemon::config::{ValidatedInfrastructure, validate_setup_input};
#[cfg(unix)]
use zterm_daemon::local_ipc::{LocalClient, serve_local};
#[cfg(unix)]
use zterm_daemon::service::DaemonService;
#[cfg(unix)]
use zterm_platform::local_unix::{DaemonLock, bind_daemon_socket, remove_own_socket};
#[cfg(unix)]
use zterm_proto::{WireKind, encode_message, v1};

#[cfg(unix)]
use state_fixture::TestState;

#[cfg(not(unix))]
fn main() {
    println!("CROSS_UID_GATE=SKIPPED_NON_UNIX");
}

#[cfg(unix)]
fn main() {
    let arguments = std::env::args().collect::<Vec<_>>();
    if arguments
        .get(1)
        .is_some_and(|value| value == "--cross-uid-client")
    {
        let socket = arguments.get(2).expect("cross-UID socket argument");
        cross_uid_client(socket);
        return;
    }
    if !cfg!(target_os = "linux") {
        println!("CROSS_UID_GATE=SKIPPED_NON_LINUX");
        return;
    }
    let privilege = std::process::Command::new("sudo")
        .args(["-n", "-u", "nobody", "true"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if !privilege.is_ok_and(|status| status.success()) {
        if std::env::var_os("CI").is_some() {
            eprintln!("CROSS_UID_GATE=FAILED_NO_NONINTERACTIVE_SUDO");
            std::process::exit(1);
        }
        println!("CROSS_UID_GATE=SKIPPED_NO_NONINTERACTIVE_SUDO");
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(cross_uid_server());
}

#[cfg(unix)]
fn cross_uid_client(socket: &str) {
    let mut stream = std::os::unix::net::UnixStream::connect(socket)
        .expect("other UID reaches test-only socket");
    let request = encode_message(
        WireKind::LocalReadinessRequest,
        1,
        0,
        &v1::LocalReadinessRequest {},
    )
    .expect("readiness frame");
    stream.write_all(&request).expect("other UID write");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("peer rejection EOF");
    assert!(
        response.is_empty(),
        "peer UID is rejected before request decode/response"
    );
}

#[cfg(unix)]
async fn cross_uid_server() {
    let state = TestState::new();
    let requested = validate_setup_input("cross-uid", ValidatedInfrastructure::OfficialN0)
        .expect("valid setup");
    let setup = bootstrap(&state.paths, &requested).expect("bootstrap");
    let lock = DaemonLock::try_acquire(&state.paths)
        .expect("lock probe")
        .expect("daemon lock");
    let listener = bind_daemon_socket(&state.paths, &lock).expect("listener");

    let temporary_root = state
        .paths
        .runtime_dir()
        .parent()
        .expect("fixture runtime parent");
    std::fs::set_permissions(temporary_root, std::fs::Permissions::from_mode(0o711))
        .expect("test-only parent traversal");
    std::fs::set_permissions(
        state.paths.runtime_dir(),
        std::fs::Permissions::from_mode(0o711),
    )
    .expect("test-only runtime traversal");
    std::fs::set_permissions(state.paths.socket(), std::fs::Permissions::from_mode(0o666))
        .expect("test-only reachable socket");

    let server = tokio::spawn(serve_local(
        listener,
        state.paths.uid(),
        Arc::new(DaemonService::new(setup)),
    ));
    let executable = std::env::current_exe().expect("harness executable");
    let status = std::process::Command::new("sudo")
        .args(["-n", "-u", "nobody", "--"])
        .arg(executable)
        .arg("--cross-uid-client")
        .arg(state.paths.socket())
        .status()
        .expect("run cross-UID client");
    assert!(
        status.success(),
        "cross-UID client harness failed: {status}"
    );

    let own_client = LocalClient::new(state.paths.socket());
    own_client
        .readiness()
        .await
        .expect("same-UID service remains healthy");
    own_client.stop(false).await.expect("server stop");
    server.await.expect("server task").expect("server result");
    remove_own_socket(&state.paths, &lock).expect("socket cleanup");
}
