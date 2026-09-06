use super::*;
use crate::bootstrap::{bootstrap, validate_committed_setup};
use crate::config::{ValidatedInfrastructure, validate_setup_input};
use crate::lifecycle::{DaemonLauncher, run_owned_daemon_listener_for_test};
use crate::local_ipc::{LocalClient, LocalIpcLimits};
use crate::operations::LocalRuntime;
use crate::service::DaemonService;
use crate::session::SessionService;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use zterm_platform::local_unix::{DaemonLock, bind_owned_daemon_socket, detach_current_process};
use zterm_platform::pty::{ExplicitPtyCommand, PtyHost, PtySize};
use zterm_platform::user_state::UserPaths;

const FIXTURE_ROOT: &str = "ZTERM_UPDATE_FIXTURE_ROOT";
const FIXTURE_ROLE: &str = "ZTERM_UPDATE_FIXTURE_ROLE";

fn paths(root: &Path) -> UserPaths {
    UserPaths::for_test(
        fs::metadata(root).expect("root metadata").uid(),
        root.into(),
        root.join(".zterm"),
        root.join("run"),
    )
}

fn runtime(root: &Path) -> LocalRuntime {
    LocalRuntime::for_test(
        paths(root),
        DaemonLauncher::for_test(root.join("zterm"), "--fixture-daemon".into()),
    )
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("fixture runtime")
        .block_on(future)
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\"'\"'"))
}

fn executable_script(root: &Path, updated: bool) -> String {
    let check = if updated {
        crate::distribution::tests::update_fixture_self_check()
    } else {
        zterm_core::release::ReleaseSelfCheck::current()
    };
    let json = serde_json::to_string(&check).expect("self check");
    let post_check = if updated {
        format!(
            "if [ \"$0\" = {} ] && [ -f {} ]; then exit 1; fi; ",
            quote(&root.join("zterm")),
            quote(&root.join("fail-post-check"))
        )
    } else {
        String::new()
    };
    let startup = if updated {
        format!(
            "if [ -f {} ]; then exit 1; fi; ",
            quote(&root.join("fail-start"))
        )
    } else {
        String::new()
    };
    let binary = std::env::current_exe().expect("test binary");
    format!(
        "#!/bin/sh\nset -eu\ncase \"$1\" in\n--internal-release-self-check) {post_check}printf '%s\\n' '{json}'; exit 0;;\n--internal-release-verify) exit 0;;\n--fixture-daemon) {startup}role=daemon;;\n--fixture-update) role=foreground;;\n--internal-update) role=worker;;\n*) exit 2;;\nesac\nexec env {FIXTURE_ROOT}={} {FIXTURE_ROLE}=\"$role\" ZTERM_UPDATE_FIXTURE_VERSION={} {} --exact update::tests::fixture_process --nocapture\n",
        quote(root),
        if updated {
            "9.1.0"
        } else {
            env!("CARGO_PKG_VERSION")
        },
        quote(&binary)
    )
}

#[test]
fn fixture_process() {
    let Some(root) = std::env::var_os(FIXTURE_ROOT).map(PathBuf::from) else {
        return;
    };
    match std::env::var(FIXTURE_ROLE).expect("fixture role").as_str() {
        "daemon" => {
            detach_current_process().expect("detach fixture daemon");
            let paths = paths(&root);
            let setup = validate_committed_setup(&paths).expect("setup");
            let lock = DaemonLock::try_acquire(&paths)
                .expect("daemon lock")
                .expect("exclusive daemon");
            let (listener, ownership) = bind_owned_daemon_socket(&paths, &lock).expect("socket");
            let cwd = root.clone();
            let sessions = SessionService::with_spawner(
                setup.device_id,
                zterm_core::ResourceLimits::default(),
                move |size, _| {
                    let command = if cwd.join("hold-session").exists() {
                        ExplicitPtyCommand::new("/bin/sleep", &cwd).arg("30")
                    } else {
                        ExplicitPtyCommand::new(cwd.join("zterm"), &cwd).arg("--fixture-update")
                    };
                    let pty = PtyHost::new()
                        .spawn(command, PtySize::new(size.rows, size.columns))
                        .expect("updater PTY");
                    Ok((pty, cwd.clone()))
                },
            );
            let mut service = DaemonService::with_sessions(setup, 17, sessions);
            service.version_override =
                Some(std::env::var("ZTERM_UPDATE_FIXTURE_VERSION").expect("version"));
            run_owned_daemon_listener_for_test(
                &paths,
                lock,
                listener,
                ownership,
                Arc::new(service),
                LocalIpcLimits::default(),
                Duration::from_secs(5),
            )
            .expect("daemon runs");
        }
        "foreground" => {
            let result = run_frontend(
                &root.join("zterm"),
                &paths(&root),
                None,
                false,
                |_: &SessionImpact| {
                    fs::write(root.join("approved"), b"yes").expect("approval marker");
                    Ok(())
                },
                |_| {},
            );

            fs::write(root.join("foreground-result"), format!("{result:?}"))
                .expect("result marker");
        }
        "worker" => {
            let paths = paths(&root);
            let stream = take_isolated_channel(paths.uid()).expect("detached updater channel");
            if root.join("disconnect-after-accept").exists() {
                // Protocol-only child failure after ownership has transferred.
                let mut channel = Channel::new(stream);
                channel.send(&Event::Ready).expect("ready");
                assert!(matches!(
                    channel.receive().expect("begin"),
                    Request::Begin { .. }
                ));
                channel.send(&Event::Handoff).expect("handoff");
                assert!(matches!(
                    channel.receive().expect("continue"),
                    Request::Continue
                ));
                channel.send(&Event::Accepted).expect("accepted");
                return;
            }
            let result = block_on(run_worker(
                &runtime(&root),
                &paths,
                stream,
                |version| async {
                    fs::write(
                        root.join("selected-version"),
                        version.unwrap_or_else(|| "latest".into()),
                    )
                    .expect("selection");
                    fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(root.join("prepare-count"))
                        .expect("preparation counter")
                        .write_all(b"1")
                        .expect("one preparation");
                    let prepared = crate::distribution::tests::prepare_update_fixture(
                        &executable_script(&root, true),
                    );
                    fs::write(
                        root.join("staged-directory"),
                        prepared
                            .candidate()
                            .parent()
                            .expect("staging parent")
                            .as_os_str()
                            .as_encoded_bytes(),
                    )
                    .expect("staging owner");
                    Ok(prepared)
                },
            ));
            fs::write(root.join("worker-result"), format!("{result:?}")).expect("worker result");
        }
        role => panic!("unknown fixture role {role}"),
    }
}

struct Fixture {
    root: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("isolated update");
        let fixture = Self { root };
        let root = fixture.root.path();
        let requested = validate_setup_input("update-test", ValidatedInfrastructure::OfficialN0)
            .expect("config");
        bootstrap(&paths(root), &requested).expect("bootstrap");
        fs::write(root.join("zterm"), executable_script(root, false)).expect("old executable");
        fs::set_permissions(root.join("zterm"), fs::Permissions::from_mode(0o700)).expect("mode");
        fixture
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = block_on(runtime(self.root.path()).stop(true));
    }
}

#[test]
fn update_completes_after_its_originating_pty_ends() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let original = fs::read(root.join("zterm")).expect("original binary");
    let identity = fs::read(paths(root).identity()).expect("identity");
    block_on(async {
        runtime(root)
            .ensure_configured_daemon()
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "{error}; daemon log: {:?}",
                    fs::read_to_string(paths(root).daemon_log())
                )
            });
        let client = LocalClient::new(paths(root).socket().to_owned());
        // The fixture PTY starts the updater itself; no fake shutdown or activation.
        let _ = client
            .create_session(
                &zterm_core::SessionName::new("update-origin").expect("name"),
                None,
                None,
            )
            .await;
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let current = fs::read(root.join("zterm")).expect("installed path");
            if current != original
                && let Ok(ready) = client.readiness().await
                && ready.version == "9.1.0"
                && root.join("worker-result").exists()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "update did not activate/start after PTY termination; approved={}, original_still_installed={}, foreground_result={:?}",
                root.join("approved").exists(),
                current == original,
                fs::read_to_string(root.join("foreground-result"))
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            client
                .status()
                .await
                .expect("updated status")
                .active_session_count,
            0
        );
        runtime(root).stop(true).await.expect("stop updated daemon");
    });
    assert!(
        fs::read_to_string(paths(root).daemon_log())
            .expect("log")
            .contains("outcome=success installed=9.1.0")
    );
    assert_eq!(
        fs::read_to_string(root.join("prepare-count")).expect("preparation"),
        "1"
    );
    assert_eq!(
        fs::read(paths(root).identity()).expect("identity retained"),
        identity
    );
}

fn hold_session(root: &Path) {
    fs::write(root.join("hold-session"), b"hold").expect("idle PTY mode");
    block_on(async {
        runtime(root)
            .ensure_configured_daemon()
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "{error}; daemon log: {:?}",
                    fs::read_to_string(paths(root).daemon_log())
                )
            });
        LocalClient::new(paths(root).socket().to_owned())
            .create_session(
                &zterm_core::SessionName::new("retained-work").expect("name"),
                None,
                None,
            )
            .await
            .expect("session admitted");
    });
}

fn assert_unchanged(root: &Path, original: &[u8]) {
    assert_eq!(
        fs::read(root.join("zterm")).expect("installed binary"),
        original
    );
    assert_eq!(
        block_on(LocalClient::new(paths(root).socket().to_owned()).status())
            .expect("daemon remains")
            .active_session_count,
        1
    );
}

fn direct_update(root: &Path) -> Result<UpdateResult, DaemonError> {
    run_frontend(
        &root.join("zterm"),
        &paths(root),
        Some("v9.1.0"),
        false,
        |_| panic!("idle update must not prompt"),
        |_| {},
    )
}

#[test]
fn external_terminal_waits_for_real_completion_and_log_survives_rotation() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let mut log = open_append(paths(root).daemon_log(), paths(root).uid()).expect("log");
    log.write_all(&vec![b'x'; 4 * 1024 * 1024])
        .expect("rotation-sized log");
    drop(log);
    let result = direct_update(root).expect("completed update");
    assert_eq!(result.installed_version, "9.1.0");
    assert!(result.daemon_started);
    assert_eq!(
        fs::read_to_string(root.join("selected-version")).expect("selected version"),
        "v9.1.0"
    );
    assert!(paths(root).logs().join("daemon.log.1").exists());
    let log = fs::read_to_string(paths(root).daemon_log()).expect("current log");
    assert!(log.contains("outcome=success installed=9.1.0 daemon_started=true"));
    assert!(!log.contains(&"x".repeat(100)));
}

#[test]
fn cancelled_confirmation_preserves_sessions_and_installation() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    hold_session(root);
    let original = fs::read(root.join("zterm")).expect("old binary");
    let error = run_frontend(
        &root.join("zterm"),
        &paths(root),
        None,
        false,
        |impact| {
            assert_eq!(impact.active_session_names, ["retained-work"]);
            Err(cancelled())
        },
        |_| {},
    )
    .expect_err("cancel update");
    assert_eq!(error.kind(), DomainErrorKind::Cancelled);
    assert_unchanged(root, &original);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.join("worker-result").exists() {
        assert!(
            Instant::now() < deadline,
            "cancelled worker must unwind staging"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let staging = fs::read_to_string(root.join("staged-directory")).expect("staging path");
    assert!(
        !Path::new(&staging).exists(),
        "cancellation cleans the candidate"
    );
}

fn child_at_handoff(root: &Path, approved: bool) -> (Child, Channel) {
    let (child, stream) =
        spawn_isolated_command(&root.join("zterm"), root, INTERNAL_UPDATE_ARGUMENT)
            .expect("updater launch");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("bounded fixture reads");
    let mut channel = Channel::new(stream);
    assert!(matches!(channel.receive().expect("ready"), Event::Ready));
    channel
        .send(&Request::Begin {
            version: None,
            approved,
        })
        .expect("begin");
    loop {
        match channel.receive().expect("prepare events") {
            Event::Progress(_) => {}
            Event::Handoff => return (child, channel),
            _ => panic!("unexpected event before handoff"),
        }
    }
}

fn wait_for_child(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while child.try_wait().expect("child status").is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("updater did not exit after control cancellation");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn lost_frontend_before_handoff_does_not_stop_approved_sessions() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    hold_session(root);
    let original = fs::read(root.join("zterm")).expect("old binary");
    let (mut child, channel) = child_at_handoff(root, true);
    drop(channel);
    wait_for_child(&mut child);
    assert_unchanged(root, &original);
}

#[test]
fn late_session_still_requires_approval_after_handoff() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    block_on(runtime(root).ensure_configured_daemon()).expect("idle daemon");
    let original = fs::read(root.join("zterm")).expect("old binary");
    let (mut child, mut channel) = child_at_handoff(root, false);
    hold_session(root);
    channel
        .send(&Request::Continue)
        .expect("handoff without force approval");
    loop {
        match channel.receive().expect("late impact") {
            Event::Accepted | Event::Progress(_) => {}
            Event::Confirm(impact) => {
                assert_eq!(impact.active_session_names, ["retained-work"]);
                break;
            }
            _ => panic!("new work must require approval"),
        }
    }
    drop(channel);
    wait_for_child(&mut child);
    assert_unchanged(root, &original);
}

#[test]
fn startup_failure_keeps_new_binary_and_records_partial_completion() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    fs::write(root.join("fail-start"), b"fail").expect("startup fault");
    let error = direct_update(root).expect_err("startup failure");
    assert!(
        error
            .detail()
            .contains("Updated zterm to 9.1.0, but the daemon could not start")
    );
    assert_eq!(
        fs::read_to_string(root.join("zterm")).expect("new binary"),
        executable_script(root, true)
    );
    let log = fs::read_to_string(paths(root).daemon_log()).expect("partial outcome log");
    assert!(log.contains("outcome=partial_completion"));
    assert!(!log.contains("outcome=success"));
}

#[test]
fn post_check_failure_restores_previous_binary_and_reports_failure() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let original = fs::read(root.join("zterm")).expect("old binary");
    fs::write(root.join("fail-post-check"), b"fail").expect("activation fault");
    direct_update(root).expect_err("post-check failure");
    assert_eq!(
        fs::read(root.join("zterm")).expect("restored binary"),
        original
    );
    let log = fs::read_to_string(paths(root).daemon_log()).expect("outcome log");
    assert!(log.contains("outcome=failed"));
    assert!(!log.contains("outcome=success") && !log.contains("outcome=partial_completion"));
}

#[test]
fn superseded_updater_cannot_replace_a_newer_activation() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let replacement = executable_script(root, true);
    let error = run_frontend(
        &root.join("zterm"),
        &paths(root),
        None,
        false,
        |_| panic!("idle"),
        |stage| {
            if stage == UpdateStage::Continuing {
                fs::write(root.join("other-update"), &replacement).expect("competing update");
                fs::set_permissions(root.join("other-update"), fs::Permissions::from_mode(0o700))
                    .expect("mode");
                fs::rename(root.join("other-update"), root.join("zterm"))
                    .expect("competing commit");
            }
        },
    )
    .expect_err("stale updater rejected");
    assert_eq!(error.kind(), DomainErrorKind::UpdateRejected);
    assert_eq!(
        fs::read_to_string(root.join("zterm")).expect("competing binary retained"),
        replacement
    );
}

#[test]
fn stalled_progress_channel_is_bounded_and_disabled_after_partial_write() {
    let (stream, _stalled_reader) = UnixStream::pair().expect("channel pair");
    let mut channel = Channel::new(stream);
    let payload = "x".repeat(MAX_CONTROL_BYTES - 10);
    let start = Instant::now();
    let mut failed = false;
    for _ in 0..100 {
        if channel.send(&payload).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed, "stalled reader must hit a bounded send");
    assert!(start.elapsed() < Duration::from_secs(3));
    let start = Instant::now();
    assert!(channel.send(&Event::Accepted).is_err());
    assert!(start.elapsed() < Duration::from_millis(100));
}

#[test]
fn failed_child_launch_leaves_daemon_and_sessions_untouched() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    hold_session(root);
    let original = fs::read(root.join("zterm")).expect("old binary");
    run_frontend(
        &root.join("missing-updater"),
        &paths(root),
        None,
        true,
        |_| panic!("no confirmation"),
        |_| {},
    )
    .expect_err("spawn failure");
    assert_unchanged(root, &original);
}

#[test]
fn update_before_setup_does_not_create_identity_or_logs() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    fs::remove_dir_all(paths(root).state_root()).expect("unconfigured fixture");
    let result = direct_update(root).expect("pre-setup update");
    assert!(!result.daemon_started);
    assert_eq!(result.installed_version, "9.1.0");
    assert!(!paths(root).state_root().exists());
}

#[test]
fn yes_approval_ends_sessions_without_reading_input() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    hold_session(root);
    let result = run_frontend(
        &root.join("zterm"),
        &paths(root),
        None,
        true,
        |_| panic!("-y must not read confirmation"),
        |_| {},
    )
    .expect("approved update");
    assert_eq!(result.ended_session_names, ["retained-work"]);
    assert!(result.daemon_started);
}

#[test]
fn accepted_channel_loss_reports_unknown_outcome_not_cancellation() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    fs::write(root.join("disconnect-after-accept"), b"disconnect").expect("protocol fault");
    let error = direct_update(root).expect_err("completion unavailable");
    assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
    assert!(error.detail().contains("may still be running"));
}
