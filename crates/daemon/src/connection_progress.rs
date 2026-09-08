//! Same-UID connection observations and bounded configured-CLI log records.

use std::{
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;
use zterm_client::progress::{ProgressEvent, ProgressHistory, ProgressObserver};
use zterm_platform::user_state::{UserPaths, open_append, validate_directory};

static NEXT_CONNECTION_LOG: AtomicU64 = AtomicU64::new(1);

pub(crate) fn recorder(paths: &UserPaths) -> (ProgressObserver, watch::Receiver<ProgressHistory>) {
    let paths = paths.clone();
    let enabled = crate::bootstrap::validate_committed_setup(&paths).is_ok();
    let connection = NEXT_CONNECTION_LOG.fetch_add(1, Ordering::Relaxed);
    ProgressObserver::channel(move |event| {
        if enabled {
            let _ = append(&paths, connection, event);
        }
    })
}

fn append(paths: &UserPaths, connection: u64, event: ProgressEvent) -> std::io::Result<()> {
    // Reopen after daemon startup rotation. Never create a configuration or log
    // directory, and never let a diagnostic failure change an operation result.
    validate_directory(paths.state_root(), paths.uid()).map_err(std::io::Error::other)?;
    validate_directory(paths.logs(), paths.uid()).map_err(std::io::Error::other)?;
    let mut file = open_append(paths.daemon_log(), paths.uid()).map_err(std::io::Error::other)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let (stage, _) = event.stage.description();
    let category = event.failure.map_or("none", |kind| kind.code());
    let severity = if event.stage == zterm_core::connection_progress::ConnectionStage::Failed {
        "WARN"
    } else {
        "INFO"
    };
    let record = format!(
        "{timestamp} {severity} connection_startup pid={} connection={connection} version={} stage={stage} category={category}\n",
        std::process::id(),
        env!("CARGO_PKG_VERSION")
    );
    file.write_all(record.as_bytes())
}

#[cfg(unix)]
pub(crate) mod local {
    use std::{future::Future, time::Instant};
    use tokio::io::{AsyncWrite, AsyncWriteExt};
    use zterm_client::progress::{CONNECTION_PROGRESS_CAPACITY, ProgressObserver};
    use zterm_proto::{WireKind, connection_stage_to_message, encode_message};

    /// Progress writing never abandons the operation whose result must survive.
    pub(crate) async fn observe<W, F, T>(
        writer: &mut W,
        request_id: u64,
        deadline: Instant,
        enabled: bool,
        operation: impl FnOnce(ProgressObserver) -> F,
    ) -> T
    where
        W: AsyncWrite + Unpin,
        F: Future<Output = T>,
    {
        if !enabled {
            return operation(ProgressObserver::default()).await;
        }
        let (observer, mut receiver) = ProgressObserver::channel(|_| {});
        let future = operation(observer.clone());
        tokio::pin!(future);
        let mut seen = 0;
        let mut sent = 0;
        loop {
            let result = tokio::select! {
                biased;
                result = &mut future => Some(result),
                changed = receiver.changed() => {
                    if changed.is_err() { return future.await; }
                    None
                }
            };
            let records = receiver.borrow_and_update().after(seen).collect::<Vec<_>>();
            for (sequence, event) in records {
                seen = sequence;
                if sent == CONNECTION_PROGRESS_CAPACITY {
                    continue;
                }
                let Some(message) = connection_stage_to_message(event.stage) else {
                    continue;
                };
                let bytes =
                    encode_message(WireKind::LocalConnectionProgress, request_id, 0, &message)
                        .expect("fixed local progress frame");
                sent += 1;
                if !matches!(
                    tokio::time::timeout_at(
                        tokio::time::Instant::from_std(deadline),
                        writer.write_all(&bytes)
                    )
                    .await,
                    Ok(Ok(()))
                ) {
                    observer.stop();
                    return match result {
                        Some(result) => result,
                        None => future.await,
                    };
                }
            }
            if let Some(result) = result {
                return result;
            }
        }
    }
}

#[cfg(all(test, unix))]
#[path = "../tests/support/state_fixture.rs"]
mod state_fixture;

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use zterm_core::{DomainErrorKind, connection_progress::ConnectionStage};
    use zterm_proto::{FrameDecoder, WireKind, connection_stage_from_message, v2};

    #[test]
    fn configured_progress_logs_reopen_after_rotation_and_stop_at_ready() {
        let state = state_fixture::TestState::new();
        let paths = &state.paths;
        let (unconfigured, _) = recorder(paths);
        unconfigured.report(ConnectionStage::Starting);
        assert!(!paths.state_root().exists());
        let config = crate::config::validate_setup_input(
            "STARTUP_TARGET_SENTINEL",
            crate::config::ValidatedInfrastructure::OfficialN0,
        )
        .expect("progress log fixture");
        crate::bootstrap::bootstrap(paths, &config).expect("configured progress fixture");
        let (observer, history) = recorder(paths);
        observer.report(ConnectionStage::Starting);
        let initial = fs::read_to_string(paths.daemon_log()).expect("read progress log");
        let archive = paths.logs().join("daemon.log.1");
        fs::rename(paths.daemon_log(), &archive).expect("rotate progress log");
        observer.report(ConnectionStage::SynchronizingTerminal);
        observer.report(ConnectionStage::TerminalReady);
        observer.stop();
        observer.clone().report(ConnectionStage::RetryingConnection);
        let final_log = fs::read_to_string(paths.daemon_log()).expect("read progress log");
        assert_eq!(
            fs::read_to_string(archive).expect("read archived log"),
            initial
        );
        assert!(initial.contains(" INFO connection_startup pid="));
        assert!(initial.contains("stage=starting category=none"));
        assert!(final_log.contains("stage=synchronizing_terminal"));
        assert!(final_log.contains("stage=terminal_ready"));
        assert_eq!(final_log.lines().count(), 2);
        let correlation = |line: &str| {
            line.split_whitespace()
                .find(|field| field.starts_with("connection="))
                .expect("progress log fixture")
                .to_owned()
        };
        assert_eq!(correlation(&initial), correlation(&final_log));
        assert!(!format!("{initial}{final_log}").contains("STARTUP_TARGET_SENTINEL"));
        assert_eq!(history.borrow().after(0).count(), 3);
        assert!(!paths.socket().exists());

        let (failure, _) = recorder(paths);
        failure.fail(DomainErrorKind::Unauthorized);
        assert!(
            fs::read_to_string(paths.daemon_log())
                .expect("progress log fixture")
                .contains("WARN connection_startup")
        );
        assert!(
            fs::read_to_string(paths.daemon_log())
                .expect("progress log fixture")
                .contains("stage=failed category=unauthorized")
        );
        // An unsafe logfile must not leak writes outside managed state or stop
        // the screen's observations; the operation owner still gets its result.
        let outside = paths.home().join("outside.log");
        fs::write(&outside, "UNCHANGED").expect("outside sentinel");
        fs::remove_file(paths.daemon_log()).expect("replace log fixture");
        std::os::unix::fs::symlink(&outside, paths.daemon_log())
            .expect("unsafe log symlink fixture");
        let (observer, history) = recorder(paths);
        observer.report(ConnectionStage::TerminalReady);
        assert_eq!(history.borrow().after(0).count(), 1);
        assert_eq!(
            fs::read_to_string(outside).expect("outside file unchanged"),
            "UNCHANGED"
        );
    }

    #[tokio::test]
    async fn local_progress_flushes_a_burst_before_the_final_result_and_survives_writer_loss() {
        let expected = [
            ConnectionStage::LookingUpAddress,
            ConnectionStage::ConnectingSecurely,
            ConnectionStage::SessionChannelReady,
        ];
        let (mut writer, mut reader) = tokio::io::duplex(4096);
        let operation = local::observe(
            &mut writer,
            17,
            std::time::Instant::now() + std::time::Duration::from_secs(1),
            true,
            |observer| async move {
                for stage in expected {
                    observer.report(stage);
                }
                42
            },
        );
        assert_eq!(operation.await, 42);
        writer.shutdown().await.expect("finish progress prefix");
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .expect("read progress prefix");
        let frames = FrameDecoder::new()
            .feed(&bytes)
            .expect("decode progress prefix");
        let stages = frames
            .iter()
            .map(|frame| {
                assert_eq!(frame.request_id, 17);
                let message: v2::LocalConnectionProgress = frame
                    .decode_message(WireKind::LocalConnectionProgress)
                    .expect("progress log fixture");
                connection_stage_from_message(message).expect("valid fixed progress stage")
            })
            .collect::<Vec<_>>();
        assert_eq!(stages, expected);

        let (mut writer, reader) = tokio::io::duplex(64);
        drop(reader);
        let (release, ready) = tokio::sync::oneshot::channel();
        let operation = local::observe(
            &mut writer,
            18,
            std::time::Instant::now() + std::time::Duration::from_secs(1),
            true,
            |observer| async move {
                observer.report(ConnectionStage::OpeningSessionChannel);
                ready.await.expect("retained operation released")
            },
        );
        let release = async {
            tokio::task::yield_now().await;
            release.send(43).expect("release submitted operation");
        };
        let (result, ()) = tokio::join!(operation, release);
        assert_eq!(
            result, 43,
            "diagnostic failure must retain the submitted operation"
        );
    }
}
