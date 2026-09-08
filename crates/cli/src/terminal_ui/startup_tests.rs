use super::outer_terminal::project_outer_child_rows;

fn startup_request(target: &str) -> TerminalRequest {
    TerminalRequest {
        kind: TerminalRequestKind::Attach {
            target: target.into(),
            selector: None,
            create_main: true,
            takeover: false,
        },
    }
}

fn replay_text(bytes: &[u8]) -> Vec<String> {
    project_outer_child_rows(bytes)
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| {
                    if cell.wide_continuation {
                        ""
                    } else if cell.contents.is_empty() {
                        " "
                    } else {
                        cell.contents.as_str()
                    }
                })
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

#[test]
fn startup_output_is_readable_bounded_and_replaced_by_the_first_session() {
    let (progress, history) = ProgressObserver::channel(|_| {});
    progress.report(ConnectionStage::InitializingTerminal);
    let startup =
        StartupProgress::new(&startup_request("local\x1b\n界"), progress.clone(), history);
    let size = TerminalSize::new(6, 80);
    let mut presenter = DesktopPresenter::default();
    let mut output = Vec::new();
    startup
        .present(&mut output, &mut presenter, size)
        .expect("initial progress");
    assert!(presenter.semantic_baseline.is_none());
    let rows = replay_text(&output);
    assert_eq!(rows[0], "Target: local界");
    assert_eq!(rows[1], "Initializing terminal");
    // A burst before the next paint must retain chronological observations.
    progress.report(ConnectionStage::TerminalInitialized);
    progress.report(ConnectionStage::CheckingLocalService);
    progress.report(ConnectionStage::LocalServiceReady);
    startup
        .present(&mut output, &mut presenter, size)
        .expect("connecting progress");
    let rows = replay_text(&output);
    assert_eq!(
        &rows[..5],
        &[
            "Target: local界",
            "Initializing terminal",
            "Terminal initialized",
            "Checking local service",
            "Local service ready"
        ]
    );
    assert!(
        rows.iter().all(|row| !row.contains("detach")
            && !row.contains("Ctrl")
            && !row.contains("elapsed"))
    );
    assert!(presenter.semantic_baseline.is_none());

    let tiny = TerminalSize::new(1, 7);
    startup
        .present(&mut output, &mut presenter, tiny)
        .expect("tiny progress");
    assert_eq!(
        presenter
            .baseline
            .as_ref()
            .expect("physical frame")
            .rows
            .len(),
        1
    );
    assert_eq!(
        presenter.baseline.as_ref().expect("physical frame").rows[&0].len(),
        7
    );
    startup
        .present(&mut output, &mut presenter, size)
        .expect("resized progress");

    let layout = ChromeLayout::new(size, ActiveScreen::Main);
    let mut snapshot = test_snapshot(layout.child, ActiveScreen::Main, Revision::new(1));
    snapshot.surface.rows[0] = test_row(
        layout.child.columns,
        "SESSION READY",
        TerminalStyle::default(),
    );
    let surface = AttachmentSurface::from_snapshot(&snapshot).expect("session surface");
    let mut viewport = ViewportController::with_layout(layout, snapshot.surface.scroll_metrics);
    let mut status = StatusRenderer::new(
        TerminalViewTarget::for_display("test", TerminalViewRoute::Local),
        size,
    );
    InactivePresentation::Synchronizing {
        surface: &surface,
        viewport: &mut viewport,
        status: &mut status,
    }
    .present(&mut output, &mut presenter, size)
    .expect("initial snapshot");
    let rows = replay_text(&output);
    assert!(rows[0].starts_with("SESSION READY"));
    assert_eq!(rows[5], "Synchronizing terminal | test");
    assert!(
        !rows
            .iter()
            .any(|row| row.contains("Target:") || row.contains("[done]") || row.contains("then ."))
    );
    assert!(presenter.semantic_baseline.is_some());
    status.initial_synchronizing = false;
    present_surface_with_writer(
        &mut output,
        &surface,
        &mut presenter,
        &viewport,
        &status,
        TerminalViewTransportState::Active,
    )
    .expect("active frame");
    assert_eq!(replay_text(&output)[5], "test | local");
}

#[derive(Clone, Default)]
struct StartupOutput(Arc<Mutex<Vec<u8>>>, Arc<AtomicBool>, Arc<AtomicBool>);

impl Write for StartupOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.1.load(Ordering::SeqCst) {
            self.2.store(true, Ordering::SeqCst);
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "startup output closed",
            ));
        }
        self.0.lock().expect("output").extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn pending_startup_keeps_progress_and_preserves_cancelled_session_results() {
    for (target, fail_output) in [("local", false), ("remote-host", false), ("local", true)] {
        let pty = openpty(
            Some(&nix::pty::Winsize {
                ws_row: 6,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            }),
            None,
        )
        .expect("startup PTY");
        let mut resize_signal = signal(SignalKind::window_change()).expect("SIGWINCH");
        let (cancel_sender, mut cancellation_receiver) = watch::channel(None);
        let input_epoch = InputEpoch::new();
        let mut stdin_pump =
            StdinPump::start(&pty.slave, input_epoch.clone()).expect("startup input");
        let input_sender = stdin_pump.sender_for_test.clone();
        let mut prefix = CommandMode::new();
        let mut input_codec = HostInputCodec::new();
        let mut host_colors = HostColors::default();
        let mut presenter = DesktopPresenter::default();
        let mut size = TerminalSize::new(6, 80);
        let (progress, history) = ProgressObserver::channel(|_| {});
        progress.report(ConnectionStage::TerminalInitialized);
        let mut startup = StartupProgress::new(&startup_request(target), progress.clone(), history);
        let output = StartupOutput::default();
        let mut writer = output.clone();
        let (release, ready) = tokio::sync::oneshot::channel();
        let session_id = SessionId::from_array([0xa1; 16]);
        let dropped = Arc::new(AtomicBool::new(false));
        let drop_probe = FutureDropProbe(Arc::clone(&dropped));
        let operation_progress = progress.clone();
        let wait = await_while_inactive(
            async move {
                operation_progress.report(ConnectionStage::CheckingLocalService);
                let _drop_probe = drop_probe;
                ready.await.expect("release prepare");
                Ok(session_id)
            },
            InactiveWaitContext {
                presentation: InactivePresentation::Startup(&mut startup),
                stdout: &pty.slave,
                resize_signal: &mut resize_signal,
                cancellation_receiver: &mut cancellation_receiver,
                stdin_pump: &mut stdin_pump,
                input_codec: &mut input_codec,
                host_colors: &mut host_colors,
                presenter: &mut presenter,
                prefix: &mut prefix,
                physical_size: &mut size,
                current_input_epoch: input_epoch.current(),
                preserve_submitted_result: true,
                report_key_events: false,
            },
            &mut writer,
        );
        let drive = async {
            input_sender
                .send(StdinEvent::Bytes {
                    geometry: 0,
                    epoch: input_epoch.current(),
                    bytes: b"must not reach the session".to_vec(),
                })
                .await
                .expect("startup typing");
            // Wait for the actual pending presentation before releasing the
            // controlled future; an eager ready future cannot pass.
            let mut observed_bytes = 0;
            loop {
                let bytes = output.0.lock().expect("output").clone();
                if bytes.len() != observed_bytes {
                    observed_bytes = bytes.len();
                    let rows = replay_text(&bytes);
                    if rows.iter().any(|row| row == "Checking local service") {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let stage = if target == "local" {
                ConnectionStage::OpeningLocalChannel
            } else {
                ConnectionStage::LookingUpAddress
            };
            progress.report(stage);
            // No input, resize or timer wakes this repaint: the real observer does.
            loop {
                let bytes = output.0.lock().expect("output").clone();
                if bytes.len() != observed_bytes {
                    observed_bytes = bytes.len();
                    if replay_text(&bytes)
                        .iter()
                        .any(|row| row == stage.description().1)
                    {
                        break;
                    }
                }
                tokio::task::yield_now().await;
            }
            if fail_output {
                output.1.store(true, Ordering::SeqCst);
                rustix::termios::tcsetwinsize(
                    &pty.master,
                    rustix::termios::Winsize {
                        ws_row: 7,
                        ws_col: 80,
                        ws_xpixel: 0,
                        ws_ypixel: 0,
                    },
                )
                .expect("resize before output failure");
                kill(Pid::this(), NixSignal::SIGWINCH).expect("request repaint");
                while !output.2.load(Ordering::SeqCst) {
                    tokio::task::yield_now().await;
                }
                assert!(
                    !dropped.load(Ordering::SeqCst),
                    "output failure must retain the submitted Session operation"
                );
                release.send(()).expect("finish after output failure");
                return;
            }
            cancel_sender
                .send(Some(TerminalSignalCancellation::new("SIGINT", Some(()))))
                .expect("cancel prepare");
            loop {
                let bytes = output.0.lock().expect("output").clone();
                if bytes.len() != observed_bytes {
                    observed_bytes = bytes.len();
                    if replay_text(&bytes)[0] == "Cancelling; waiting for session result" {
                        break;
                    }
                }
                tokio::task::yield_now().await;
            }
            let rows = replay_text(&output.0.lock().expect("output"));
            assert_eq!(rows[0], "Cancelling; waiting for session result");
            assert_eq!(rows[1], format!("Target: {target}"));
            release.send(()).expect("finish submitted session");
        };
        let (result, ()) =
            tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(wait, drive) })
                .await
                .expect("bounded pending startup");
        match result.expect("prepare result") {
            InactiveWait::CompletedAfterCancellation {
                value,
                cancellation,
            } => {
                assert_eq!(value, session_id);
                if fail_output {
                    assert!(matches!(
                        &cancellation,
                        InactiveCancellation::PresentationFailure(_)
                    ));
                    let error = inactive_cancellation_result(cancellation, Some(value))
                        .expect_err("visible output error");
                    assert!(error.to_string().contains(&session_id.to_string()));
                } else {
                    assert!(matches!(cancellation, InactiveCancellation::Signal(_)));
                }
            }
            _ => panic!("lost the submitted Session outcome"),
        }
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(
            input_epoch.current(),
            0,
            "startup must not admit a keyboard epoch"
        );
        assert!(presenter.semantic_baseline.is_none());
        stdin_pump.shutdown().expect("stop startup input");
    }
}

#[test]
fn startup_outcomes_distinguish_session_end_cancel_and_failure_and_ignore_later_exit() {
    let cases = [
        (
            Ok(TerminalCompletion::SessionEnded(
                TerminalViewEndReason::NaturalExit,
            )),
            ConnectionStage::SessionEnded,
            None,
        ),
        (
            Ok(TerminalCompletion::Detached),
            ConnectionStage::Cancelled,
            None,
        ),
        (
            Err(CliError::Io("PRIVATE_ERROR_SENTINEL".into())),
            ConnectionStage::Failed,
            Some(ProgressFailure::TerminalIo),
        ),
        (
            Err(CliError::Daemon(DaemonError::new(
                DomainErrorKind::Unauthorized,
                "PRIVATE_ERROR_SENTINEL",
            ))),
            ConnectionStage::Failed,
            Some(ProgressFailure::Domain(DomainErrorKind::Unauthorized)),
        ),
    ];
    for (result, stage, failure) in cases {
        let (observer, history) = ProgressObserver::channel(|_| {});
        finish_startup_progress(&observer, &result);
        let events = history
            .borrow()
            .after(0)
            .map(|(_, event)| event)
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].stage, stage);
        assert_eq!(events[0].failure, failure);
        assert!(!format!("{events:?}").contains("PRIVATE_ERROR_SENTINEL"));
        finish_startup_progress(&observer, &Ok(TerminalCompletion::Detached));
        assert_eq!(
            history.borrow().after(0).count(),
            1,
            "one startup outcome only"
        );
    }
    let (observer, history) = ProgressObserver::channel(|_| {});
    observer.report(ConnectionStage::TerminalReady);
    observer.stop();
    finish_startup_progress(&observer, &Ok(TerminalCompletion::Detached));
    assert_eq!(
        history
            .borrow()
            .after(0)
            .next()
            .expect("ready outcome")
            .1
            .stage,
        ConnectionStage::TerminalReady
    );
    assert_eq!(history.borrow().after(0).count(), 1);
}
