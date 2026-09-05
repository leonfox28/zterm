// Application-neutral, planning-only schedule using the real SessionService.
#[test]
fn planning_probe_queued_delta_after_resize_does_not_ack_an_active_attachment() {
    use zterm_core::ResourceLimits;
    use zterm_daemon::session::AttachmentUpdate;

    let fixture = planning_session_fixture::Fixture::new(ResourceLimits::default()).unwrap();
    let physical = TerminalSize::new(24, 80);
    let layout = ChromeLayout::new(physical, ActiveScreen::Main);
    let prepared = fixture
        .service
        .prepare_attach(fixture.principal, None, true, false, Some(layout.child))
        .unwrap();
    planning_session_fixture::activate(&prepared).unwrap();
    let mut surface = AttachmentSurface::from_snapshot(&prepared.snapshot).unwrap();
    prepared
        .attachment
        .write_input(b"printf 'queued-output\\n'\n")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let queued = loop {
        match prepared.attachment.next_update().unwrap() {
            Some(AttachmentUpdate::Delta(delta)) => break delta,
            Some(AttachmentUpdate::Snapshot(snapshot)) => {
                prepared
                    .attachment
                    .snapshot_applied(snapshot.revision)
                    .unwrap();
                surface = AttachmentSurface::from_snapshot(&snapshot).unwrap();
            }
            None => {
                assert!(
                    Instant::now() < deadline,
                    "fixture must produce a real queued delta"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    };

    // The server has already emitted an ordinary delta. The UI submits resize
    // before consuming it; no replacement snapshot has been produced yet.
    let resized_physical = TerminalSize::new(24, 81);
    let resized_layout = ChromeLayout::new(resized_physical, ActiveScreen::Main);
    let mut coalescer = ResizeCoalescer::new(layout.child);
    let resize = coalescer
        .observe(resized_layout.child, TerminalViewTransportState::Active)
        .unwrap();
    prepared.attachment.resize(resize).unwrap();
    let state = TerminalViewTransportState::Synchronizing;
    let lifecycle = prepared
        .attachment
        .lifecycle_watch()
        .unwrap()
        .borrow()
        .clone();
    assert!(matches!(
        lifecycle,
        zterm_daemon::session::AttachmentLifecycle::Active { .. }
    ));

    let mut viewport =
        ViewportController::with_layout(resized_layout, surface.surface.scroll_metrics);
    let mut presenter = DesktopPresenter::default();
    let mut selection = SelectionController::default();
    let status = StatusRenderer::new(
        TerminalViewTarget::for_display("probe", TerminalViewRoute::Local),
        resized_physical,
    );
    let mut output = ViewportFrameWriter::default();
    let acknowledges = delta_acknowledges_existing_sync(state);
    assert_eq!(
        apply_delta_with_writer(
            &mut output,
            &mut surface,
            &mut presenter,
            &queued,
            &mut viewport,
            &mut selection,
            &status,
            state
        )
        .unwrap(),
        DeltaRender::Applied
    );
    let error = if acknowledges {
        prepared
            .attachment
            .snapshot_applied(queued.to_revision)
            .err()
    } else {
        None
    };
    fixture.service.shutdown().unwrap();
    eprintln!(
        "ordinary_delta={}..{} resize={}x{} inferred_ack={acknowledges} server_error={error:?}",
        queued.from_revision.get(),
        queued.to_revision.get(),
        resize.rows,
        resize.columns
    );
    assert!(
        error.is_none(),
        "an already-emitted ordinary delta was misidentified as a synchronization barrier: {error:?}"
    );
}
