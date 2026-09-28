//! Same-UID connection observations and bounded configured-CLI log records.

use tokio::sync::watch;
use zterm_client::progress::{ProgressHistory, ProgressObserver};
use zterm_platform::user_state::UserPaths;

pub(crate) fn recorder(paths: &UserPaths) -> (ProgressObserver, watch::Receiver<ProgressHistory>) {
    let log = crate::diagnostics::recorder(paths);
    recorder_with_log(log)
}

fn recorder_with_log(
    log: zterm_diagnostics::Recorder,
) -> (ProgressObserver, watch::Receiver<ProgressHistory>) {
    use zterm_core::connection_progress::ConnectionStage;
    use zterm_diagnostics::{Event, Kind, Level, Operation};
    let operation = Operation::default();
    ProgressObserver::channel(move |event| {
        let terminal = matches!(
            event.stage,
            ConnectionStage::Starting
                | ConnectionStage::TerminalReady
                | ConnectionStage::Failed
                | ConnectionStage::Cancelled
                | ConnectionStage::SessionEnded
        );
        let level = if event.failure.is_some() {
            Level::Warn
        } else if terminal {
            Level::Info
        } else {
            Level::Debug
        };
        let mut record = Event::new(Kind::ConnectionStartup)
            .level(level)
            .operation(&operation)
            .connection(operation.id())
            .startup(event.stage);
        if let Some(error) = event.failure {
            use zterm_client::progress::ProgressFailure;
            use zterm_diagnostics::FrontendFailure;
            record = match error {
                ProgressFailure::Domain(error) => record.error(error),
                ProgressFailure::TerminalIo => record.frontend_error(FrontendFailure::TerminalIo),
                ProgressFailure::InvalidUsage => {
                    record.frontend_error(FrontendFailure::InvalidUsage)
                }
                ProgressFailure::TerminalDriver => {
                    record.frontend_error(FrontendFailure::TerminalDriver)
                }
            };
        }
        log.record(record);
    })
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
        let log = crate::diagnostics::recorder(paths);
        let (observer, history) = recorder_with_log(log.clone());
        observer.report(ConnectionStage::Starting);
        assert!(log.flush(std::time::Duration::from_secs(2)));
        let initial = fs::read_to_string(paths.daemon_log()).expect("read progress log");
        let archive = paths.logs().join("daemon.log.1");
        fs::rename(paths.daemon_log(), &archive).expect("rotate progress log");
        observer.report(ConnectionStage::SynchronizingTerminal);
        observer.report(ConnectionStage::TerminalReady);
        observer.stop();
        observer.clone().report(ConnectionStage::RetryingConnection);
        assert!(log.flush(std::time::Duration::from_secs(2)));
        let final_log = fs::read_to_string(paths.daemon_log()).expect("read progress log");
        assert_eq!(
            fs::read_to_string(archive).expect("read archived log"),
            initial
        );
        let initial_record =
            zterm_diagnostics::Record::decode(initial.trim().as_bytes()).expect("structured start");
        let final_record = zterm_diagnostics::Record::decode(final_log.trim().as_bytes())
            .expect("structured ready");
        assert_eq!(initial_record.level, zterm_diagnostics::Level::Info);
        assert_eq!(
            initial_record.fields.startup_stage.as_deref(),
            Some("starting")
        );
        assert_eq!(
            final_record.fields.startup_stage.as_deref(),
            Some("terminal_ready")
        );
        assert_eq!(
            final_log.lines().count(),
            1,
            "intermediate stages require opt-in detail"
        );
        assert_eq!(
            initial_record.fields.connection_id,
            final_record.fields.connection_id
        );
        assert!(!format!("{initial}{final_log}").contains("STARTUP_TARGET_SENTINEL"));
        assert_eq!(history.borrow().after(0).count(), 3);
        assert!(!paths.socket().exists());

        let (failure, _) = recorder_with_log(log.clone());
        failure.fail(DomainErrorKind::Unauthorized);
        assert!(log.flush(std::time::Duration::from_secs(2)));
        let failed = fs::read_to_string(paths.daemon_log()).expect("progress log fixture");
        let failed = zterm_diagnostics::Record::decode(
            failed.lines().last().expect("failed record").as_bytes(),
        )
        .expect("typed record");
        assert_eq!(failed.level, zterm_diagnostics::Level::Warn);
        assert_eq!(failed.fields.category.as_deref(), Some("unauthorized"));
        // An unsafe logfile must not leak writes outside managed state or stop
        // the screen's observations; the operation owner still gets its result.
        let outside = paths.home().join("outside.log");
        fs::write(&outside, "UNCHANGED").expect("outside sentinel");
        fs::remove_file(paths.daemon_log()).expect("replace log fixture");
        std::os::unix::fs::symlink(&outside, paths.daemon_log())
            .expect("unsafe log symlink fixture");
        let (observer, history) = recorder_with_log(log.clone());
        observer.report(ConnectionStage::TerminalReady);
        assert!(!log.flush(std::time::Duration::from_secs(2)));
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
