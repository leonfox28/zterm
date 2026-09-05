use nix::pty::openpty;
use zterm_core::ResourceLimits;
use zterm_daemon::bootstrap::bootstrap;
use zterm_daemon::config::{ValidatedInfrastructure, validate_setup_input};
use zterm_daemon::lifecycle::DaemonLauncher;
use zterm_daemon::local_ipc::serve_local;
use zterm_daemon::service::DaemonService;
use zterm_daemon::session::SessionService;
use zterm_platform::local_unix::{DaemonLock, bind_daemon_socket};
use zterm_platform::pty::{ExplicitPtyCommand, PtyHost, PtySize};
use zterm_platform::user_state::UserPaths;

#[derive(Clone, Copy, Debug)]
enum ResizeTrigger {
    ResumeDelta,
    UnprocessedResize,
    Physical,
    DeferredSnapshot,
}

// Runs the actual UI event handler and command driver against SessionWireServer
// and SessionService. Holding already-received updates controls the UI schedule;
// it does not add timing hooks or change target synchronization rules.
#[tokio::test]
async fn queued_deltas_across_resize_do_not_create_snapshot_ack_cascades() {
    for trigger in [
        ResizeTrigger::ResumeDelta,
        ResizeTrigger::UnprocessedResize,
        ResizeTrigger::Physical,
        ResizeTrigger::DeferredSnapshot,
    ] {
        tokio::time::timeout(Duration::from_secs(10), queued_delta_resize_case(trigger))
            .await
            .expect("bounded queued-delta regression");
    }
}

async fn queued_delta_resize_case(trigger: ResizeTrigger) {
    let temporary = tempfile::tempdir().expect("synchronization fixture operation succeeds");
    let home = temporary.path().join("home");
    std::fs::create_dir(&home).expect("synchronization fixture operation succeeds");
    let paths = UserPaths::for_test(
        nix::unistd::Uid::effective().as_raw(),
        home.clone(),
        home.join(".zterm"),
        temporary.path().join("run"),
    );
    let config = validate_setup_input("sync-test", ValidatedInfrastructure::OfficialN0)
        .expect("synchronization fixture operation succeeds");
    let setup = bootstrap(&paths, &config).expect("synchronization fixture operation succeeds");
    let lock = DaemonLock::try_acquire(&paths)
        .expect("synchronization fixture operation succeeds")
        .expect("synchronization fixture operation succeeds");
    let listener =
        bind_daemon_socket(&paths, &lock).expect("synchronization fixture operation succeeds");
    let sessions = SessionService::with_spawner(
        setup.device_id,
        ResourceLimits::default(),
        move |size, _| {
            let command = ExplicitPtyCommand::new("/bin/sh", &home).arg("-c").arg(
            "while IFS= read -r line; do case \"$line\" in switch) printf '\\033[?1049h';; *) printf '%s\\n' \"$line\";; esac; done"
        );
            let pty = PtyHost::new()
                .spawn(command, PtySize::new(size.rows, size.columns))
                .map_err(|error| {
                    DaemonError::new(DomainErrorKind::InvalidWorkingDirectory, error.to_string())
                })?;
            Ok((pty, home.clone()))
        },
    );
    let service = Arc::new(DaemonService::with_sessions(setup, 1, sessions.clone()));
    let server = tokio::spawn(serve_local(listener, paths.uid(), service));
    let runtime = LocalRuntime::for_test(
        paths,
        DaemonLauncher::for_test("/must-not-launch".into(), "--unused".into()),
    );
    let physical_size = TerminalSize::new(5, 24);
    let layout = ChromeLayout::new(physical_size, ActiveScreen::Main);
    let prepared = runtime
        .attach("local", None, true, false, Some(layout.child))
        .await
        .expect("synchronization fixture operation succeeds");
    let session_id = prepared.session_id();
    let snapshot = prepared.initial_snapshot().clone();
    let target = prepared.target().clone();
    let (events, writer) = prepared
        .acknowledge_initial()
        .await
        .expect("synchronization fixture operation succeeds")
        .split();
    let pty = openpty(None, None).expect("synchronization fixture operation succeeds");
    let input_epoch = InputEpoch::new();
    let mut ui = TerminalUiSession {
        session_id,
        events,
        writer,
        current_input_epoch: input_epoch.current(),
        stdin_pump: StdinPump::start(&pty.slave, input_epoch.clone())
            .expect("synchronization fixture operation succeeds"),
        input_epoch,
        prefix: PrefixParser::new(None),
        transport_state: TerminalViewTransportState::Active,
        resize_coalescer: ResizeCoalescer::new(layout.child),
        physical_size,
        surface: AttachmentSurface::from_snapshot(&snapshot)
            .expect("synchronization fixture operation succeeds"),
        presenter: DesktopPresenter::default(),
        selection: SelectionController::default(),
        viewport: ViewportController::with_layout(layout, snapshot.surface.scroll_metrics),
        status_renderer: StatusRenderer::new(target, physical_size),
        viewport_pacer: ViewportPresentationPacer::default(),
        sync_requested: false,
        input_codec: HostInputCodec::new(),
        copy_key_lease: CopyKeyLease::default(),
        deferred_active: false,
    };
    assert!(matches!(
        ui.events
            .read_event()
            .await
            .expect("synchronization fixture operation succeeds"),
        Some(TerminalViewEvent::TransportState(
            TerminalViewTransportState::Active
        ))
    ));

    let queued = match trigger {
        ResizeTrigger::ResumeDelta => {
            ui.writer
                .write_input(b"resume-baseline\n".to_vec())
                .await
                .expect("synchronization fixture operation succeeds");
            let _ = next_delta(&mut ui).await;
            ui.writer
                .request_sync(ui.surface.revision())
                .await
                .expect("synchronization fixture operation succeeds");
            let snapshot = next_snapshot(&mut ui).await;
            ui.viewport
                .begin_resume(Vec::new())
                .expect("synchronization fixture operation succeeds");
            ui.transport_state = TerminalViewTransportState::Synchronizing;
            // The remote correlation/variant origin is tested in SessionClient.
            // Here the real target supplies an Awaiting revision and the UI must
            // apply, present and ACK the explicit delta barrier, releasing resume.
            let delta = TerminalSurfaceDelta {
                from_revision: ui.surface.revision(),
                to_revision: snapshot.revision,
                size: snapshot.surface.size,
                active_screen: snapshot.surface.active_screen,
                row_patches: snapshot
                    .surface
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(row, cells)| TerminalSurfaceRowPatch {
                        row: u16::try_from(row)
                            .expect("synchronization fixture operation succeeds"),
                        replacement: cells.clone(),
                    })
                    .collect(),
                cursor: snapshot.surface.cursor,
                modes: snapshot.surface.modes,
                scroll_metrics: snapshot.surface.scroll_metrics,
            };
            ui.handle_event(&pty.slave, TerminalViewEvent::ResumeDelta(delta))
                .await
                .expect("synchronization fixture operation succeeds");
            assert!(ui.viewport.is_resume_pending());
            None
        }

        ResizeTrigger::UnprocessedResize => {
            ui.writer
                .write_input(b"queued-before-target-resize\n".to_vec())
                .await
                .expect("synchronization fixture operation succeeds");
            let queued = next_delta(&mut ui).await;
            // Model the frontend resize fence before the target processes resize:
            // SessionService is still Active. Ordinary delta ACK is illegal here.
            ui.transport_state = TerminalViewTransportState::Synchronizing;
            Some(queued)
        }
        ResizeTrigger::Physical => {
            ui.writer
                .write_input(b"queued-before-resize\n".to_vec())
                .await
                .expect("synchronization fixture operation succeeds");
            let queued = next_delta(&mut ui).await;
            ui.physical_size.columns += 1;
            ui.status_renderer.resize(ui.physical_size);
            let layout = ChromeLayout::new(ui.physical_size, ActiveScreen::Main);
            ui.viewport.set_layout(layout);
            let size = ui
                .resize_coalescer
                .observe(layout.child, ui.transport_state)
                .expect("synchronization fixture operation succeeds");
            ui.writer
                .resize(size)
                .await
                .expect("synchronization fixture operation succeeds");
            ui.transition_transport(&pty.slave, TerminalViewTransportState::Synchronizing)
                .await
                .expect("synchronization fixture operation succeeds");
            Some(queued)
        }
        ResizeTrigger::DeferredSnapshot => {
            ui.writer
                .write_input(b"switch\n".to_vec())
                .await
                .expect("synchronization fixture operation succeeds");
            let snapshot = next_snapshot(&mut ui).await;
            assert_eq!(snapshot.surface.active_screen, ActiveScreen::Alternate);
            ui.handle_event(&pty.slave, TerminalViewEvent::Snapshot(snapshot))
                .await
                .expect("synchronization fixture operation succeeds");
            // Receive the ordinary delta before consuming the queued Active.
            ui.writer
                .write_input(b"queued-after-snapshot\n".to_vec())
                .await
                .expect("synchronization fixture operation succeeds");
            let queued = next_delta(&mut ui).await;
            ui.handle_event(
                &pty.slave,
                TerminalViewEvent::TransportState(TerminalViewTransportState::Active),
            )
            .await
            .expect("synchronization fixture operation succeeds");
            Some(queued)
        }
    };
    assert_eq!(
        ui.transport_state,
        TerminalViewTransportState::Synchronizing,
        "{trigger:?}"
    );
    // The target is now Awaiting the resize snapshot. A stale delta ACK would
    // enqueue a replacement snapshot, then make the second exact ACK fail.
    let replacement = if matches!(
        trigger,
        ResizeTrigger::UnprocessedResize | ResizeTrigger::ResumeDelta
    ) {
        None
    } else {
        Some(next_snapshot(&mut ui).await)
    };
    if let Some(queued) = queued {
        ui.handle_event(&pty.slave, TerminalViewEvent::Delta(queued))
            .await
            .expect("synchronization fixture operation succeeds");
    }
    if let Some(replacement) = replacement {
        ui.handle_event(&pty.slave, TerminalViewEvent::Snapshot(replacement))
            .await
            .expect("synchronization fixture operation succeeds");
    }
    ui.writer
        .write_input(b"AFTER-RESIZE-ACK\n".to_vec())
        .await
        .expect("synchronization fixture operation succeeds");
    loop {
        let event = ui
            .events
            .read_event()
            .await
            .expect("synchronization fixture operation succeeds")
            .expect("attachment stays live");
        assert!(
            !matches!(event, TerminalViewEvent::Snapshot(_)),
            "ordinary delta caused a duplicate snapshot for {trigger:?}"
        );
        ui.handle_event(&pty.slave, event)
            .await
            .expect("synchronization fixture operation succeeds");
        let text: String = ui
            .surface
            .surface
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .map(|cell| cell.contents.as_str())
            .collect();
        if text.contains("AFTER-RESIZE-ACK") {
            break;
        }
    }
    if matches!(trigger, ResizeTrigger::ResumeDelta) {
        assert!(
            ui.viewport.is_live(),
            "explicit resume delta must complete the input fence"
        );
    }
    ui.stdin_pump
        .shutdown()
        .expect("synchronization fixture operation succeeds");
    ui.writer
        .detach()
        .await
        .expect("synchronization fixture operation succeeds");
    drop(ui);
    server.abort();
    let _ = server.await;
    tokio::task::spawn_blocking(move || sessions.shutdown())
        .await
        .expect("synchronization fixture operation succeeds")
        .expect("synchronization fixture operation succeeds");
}

async fn next_delta(ui: &mut TerminalUiSession) -> TerminalSurfaceDelta {
    loop {
        match ui
            .events
            .read_event()
            .await
            .expect("synchronization fixture operation succeeds")
            .expect("queued update stream stays open")
        {
            TerminalViewEvent::Delta(delta) => return delta,
            TerminalViewEvent::TransportState(_) | TerminalViewEvent::SyncRequired { .. } => {}
            event => panic!("expected ordinary delta, got {event:?}"),
        }
    }
}

async fn next_snapshot(ui: &mut TerminalUiSession) -> TerminalSurfaceSnapshot {
    loop {
        match ui
            .events
            .read_event()
            .await
            .expect("synchronization fixture operation succeeds")
            .expect("snapshot stream stays open")
        {
            TerminalViewEvent::Snapshot(snapshot) => return snapshot,
            TerminalViewEvent::TransportState(_)
            | TerminalViewEvent::SyncRequired { .. }
            | TerminalViewEvent::Delta(_) => {}
            event => panic!("expected replacement snapshot, got {event:?}"),
        }
    }
}
