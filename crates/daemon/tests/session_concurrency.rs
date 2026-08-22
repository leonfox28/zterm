//! Replay singleflight and atomic session-name reservation regressions.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use zterm_core::{
    AttachmentId, DeviceId, DomainErrorKind, OperationId, ResourceLimits, SessionName,
};
use zterm_daemon::error::DaemonError;
use zterm_daemon::session::SessionService;
use zterm_platform::pty::{ExplicitPtyCommand, PtyHost, PtySession, PtySize};

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn unrelated_operation_keys_overlap_while_same_key_joins_exactly() -> Result<(), String> {
    let temporary = tempfile::tempdir().map_err(display)?;
    let cwd = temporary.path().to_path_buf();
    let shell = shell_path()?;
    let spawn_count = Arc::new(AtomicUsize::new(0));
    let (slow_entered, entered) = mpsc::sync_channel(1);
    let (release_slow, release) = mpsc::sync_channel(1);
    let release = Arc::new(Mutex::new(release));
    let service =
        SessionService::with_spawner(DeviceId::from_array([31; 32]), ResourceLimits::default(), {
            let cwd = cwd.clone();
            let shell = shell.clone();
            let spawn_count = Arc::clone(&spawn_count);
            let release = Arc::clone(&release);
            move |size, requested| {
                let spawn_index = spawn_count.fetch_add(1, Ordering::AcqRel);
                if spawn_index == 0 {
                    slow_entered
                        .send(())
                        .map_err(|_| cancelled("slow-create observer disappeared"))?;
                    release
                        .lock()
                        .map_err(|_| cancelled("slow-create release lock poisoned"))?
                        .recv()
                        .map_err(|_| cancelled("slow-create release disappeared"))?;
                }
                spawn_shell(&shell, requested.unwrap_or(&cwd), size)
            }
        });
    let principal = service.local_principal(AttachmentId::from_array([4; 16]));
    let lease = service.issue_operation_lease(principal).map_err(display)?;
    let slow_operation = OperationId { lease, sequence: 1 };
    let (slow_result, slow_response) = mpsc::sync_channel(1);
    let slow_service = service.clone();
    let slow_thread = thread::spawn(move || {
        let result = slow_service.create(
            principal,
            slow_operation,
            SessionName::new("slow").expect("valid slow name"),
            None,
            None,
        );
        let _ = slow_result.send(result);
    });
    entered
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| "slow create never reached its isolated side effect".to_owned())?;

    let (duplicate_result, duplicate_response) = mpsc::sync_channel(1);
    let duplicate_service = service.clone();
    let duplicate_thread = thread::spawn(move || {
        let result = duplicate_service.create(
            principal,
            slow_operation,
            SessionName::new("slow").expect("valid duplicate name"),
            None,
            None,
        );
        let _ = duplicate_result.send(result);
    });

    let (fast_result, fast_response) = mpsc::sync_channel(1);
    let fast_service = service.clone();
    let fast_thread = thread::spawn(move || {
        let result = fast_service.create(
            principal,
            OperationId { lease, sequence: 2 },
            SessionName::new("fast").expect("valid fast name"),
            None,
            None,
        );
        let _ = fast_result.send(result);
    });

    let fast = match fast_response.recv_timeout(TEST_TIMEOUT) {
        Ok(result) => result.map_err(display)?,
        Err(error) => {
            let _ = release_slow.send(());
            let _ = slow_thread.join();
            let _ = duplicate_thread.join();
            let _ = fast_thread.join();
            return Err(format!(
                "unrelated operation key waited behind the slow key: {error}"
            ));
        }
    };
    assert_eq!(fast.name.as_str(), "fast");
    assert!(matches!(
        duplicate_response.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));

    release_slow
        .send(())
        .map_err(|_| "slow create release receiver disappeared".to_owned())?;
    let slow = slow_response
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| "slow create did not finish after release".to_owned())?
        .map_err(display)?;
    let duplicate = duplicate_response
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| "same-key waiter did not receive the completed result".to_owned())?
        .map_err(display)?;
    assert_eq!(duplicate, slow);
    let mismatch = service
        .create(
            principal,
            slow_operation,
            SessionName::new("must-not-run").expect("valid mismatched name"),
            None,
            None,
        )
        .expect_err("same operation ID with different payload is rejected");
    assert_eq!(mismatch.kind(), DomainErrorKind::OperationOutcomeUnknown);
    slow_thread
        .join()
        .map_err(|_| "slow create thread panicked".to_owned())?;
    duplicate_thread
        .join()
        .map_err(|_| "duplicate create thread panicked".to_owned())?;
    fast_thread
        .join()
        .map_err(|_| "fast create thread panicked".to_owned())?;
    assert_eq!(slow.name.as_str(), "slow");
    assert_eq!(spawn_count.load(Ordering::Acquire), 2);
    assert_eq!(service.list().map_err(display)?.len(), 2);
    service.shutdown().map_err(display)?;
    Ok(())
}

#[test]
fn create_reservation_prevents_a_concurrent_rename_from_publishing_the_same_name()
-> Result<(), String> {
    let temporary = tempfile::tempdir().map_err(display)?;
    let cwd = temporary.path().to_path_buf();
    let shell = shell_path()?;
    let spawn_count = Arc::new(AtomicUsize::new(0));
    let (reserved, reservation_observed) = mpsc::sync_channel(1);
    let (release_create, release) = mpsc::sync_channel(1);
    let release = Arc::new(Mutex::new(release));
    let service =
        SessionService::with_spawner(DeviceId::from_array([32; 32]), ResourceLimits::default(), {
            let cwd = cwd.clone();
            let shell = shell.clone();
            let spawn_count = Arc::clone(&spawn_count);
            let release = Arc::clone(&release);
            move |size, requested| {
                if spawn_count.fetch_add(1, Ordering::AcqRel) == 1 {
                    reserved
                        .send(())
                        .map_err(|_| cancelled("name-reservation observer disappeared"))?;
                    release
                        .lock()
                        .map_err(|_| cancelled("name-reservation release lock poisoned"))?
                        .recv()
                        .map_err(|_| cancelled("name-reservation release disappeared"))?;
                }
                spawn_shell(&shell, requested.unwrap_or(&cwd), size)
            }
        });
    let principal = service.local_principal(AttachmentId::from_array([5; 16]));
    let lease = service.issue_operation_lease(principal).map_err(display)?;
    let source = service
        .create(
            principal,
            OperationId { lease, sequence: 1 },
            SessionName::new("source").expect("valid source name"),
            None,
            None,
        )
        .map_err(display)?;

    let (created, create_result) = mpsc::sync_channel(1);
    let create_service = service.clone();
    let create_thread = thread::spawn(move || {
        let result = create_service.create(
            principal,
            OperationId { lease, sequence: 2 },
            SessionName::new("reserved-target").expect("valid target name"),
            None,
            None,
        );
        let _ = created.send(result);
    });
    reservation_observed
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| "create never reached the reserved-name spawn boundary".to_owned())?;

    let rename = service
        .rename(
            principal,
            OperationId { lease, sequence: 3 },
            source.session_id,
            SessionName::new("reserved-target").expect("valid target name"),
        )
        .expect_err("rename cannot claim a provisionally reserved create name");
    assert_eq!(rename.kind(), DomainErrorKind::SessionAlreadyExists);

    release_create
        .send(())
        .map_err(|_| "reserved create release receiver disappeared".to_owned())?;
    let target = create_result
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| "reserved create did not publish after release".to_owned())?
        .map_err(display)?;
    create_thread
        .join()
        .map_err(|_| "reserved create thread panicked".to_owned())?;
    assert_eq!(target.name.as_str(), "reserved-target");
    let sessions = service.list().map_err(display)?;
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions
            .iter()
            .filter(|summary| summary.name.as_str() == "reserved-target")
            .count(),
        1
    );
    assert!(
        sessions
            .iter()
            .any(|summary| summary.name.as_str() == "source")
    );
    service.shutdown().map_err(display)?;
    Ok(())
}

fn spawn_shell(
    shell: &Path,
    working_directory: &Path,
    size: zterm_core::terminal::TerminalSize,
) -> Result<(PtySession, PathBuf), DaemonError> {
    let session = PtyHost::new()
        .spawn(
            ExplicitPtyCommand::new(shell, working_directory).arg("-i"),
            PtySize::new(size.rows, size.columns),
        )
        .map_err(|error| cancelled(error.to_string()))?;
    Ok((session, working_directory.to_path_buf()))
}

fn shell_path() -> Result<PathBuf, String> {
    ["/bin/sh", "/usr/bin/sh"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .ok_or_else(|| "no absolute POSIX shell fixture is available".to_owned())
}

fn cancelled(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::Cancelled, detail)
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}
