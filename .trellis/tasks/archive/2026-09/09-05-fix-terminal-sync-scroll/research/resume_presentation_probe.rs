// Planning-only regression injected into a byte-for-byte copy of the CLI module.
// This must fail on v0.1.16: it asserts actual committed row content, not chrome.
#[test]
fn planning_probe_resume_paints_latest_rows_before_any_click() {
    let mut missing = Vec::new();
    for route in [TerminalViewRoute::Local, TerminalViewRoute::Remote] {
        for new_output in [false, true] {
            let physical = TerminalSize::new(5, 24);
            let layout = ChromeLayout::new(physical, ActiveScreen::Main);
            let mut snapshot = test_snapshot(layout.child, ActiveScreen::Main, Revision::new(2));
            for (index, row) in snapshot.surface.rows.iter_mut().enumerate() {
                *row = test_row(
                    layout.child.columns,
                    &format!("live-{index}"),
                    TerminalStyle::default(),
                );
            }
            let mut surface = AttachmentSurface::from_snapshot(&snapshot).unwrap();
            let mut viewport =
                ViewportController::with_layout(layout, snapshot.surface.scroll_metrics);
            let status =
                StatusRenderer::new(TerminalViewTarget::for_display("probe", route), physical);
            let mut presenter = DesktopPresenter::default();
            let mut output = ViewportFrameWriter::default();
            present_surface_with_writer(
                &mut output,
                &surface,
                &mut presenter,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .unwrap();
            viewport.observe_presentation();

            install_test_history_window(&mut viewport);
            let _ = viewport.navigate(true, 1);
            assert_eq!(viewport.window_cache.desired_offset_from_bottom(), 2);
            present_surface_with_writer(
                &mut output,
                &surface,
                &mut presenter,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .unwrap();
            viewport.observe_presentation();
            let history_rows = presenter.baseline.as_ref().unwrap().rows.clone();

            assert!(matches!(
                viewport.navigate(false, 2),
                ViewportEffect::Resume
            ));
            viewport.observe_sync_required();
            if new_output {
                snapshot.revision = Revision::new(3);
                snapshot.surface.scroll_metrics.as_mut().unwrap().revision = snapshot.revision;
                for row in 2..4 {
                    snapshot.surface.rows[row] = test_row(
                        layout.child.columns,
                        &format!("new-tail-{row}"),
                        TerminalStyle::default(),
                    );
                }
            }
            // Same sequence as TerminalUiSession::handle_event(Snapshot).
            viewport.observe_snapshot(snapshot.surface.scroll_metrics);
            viewport.set_layout(layout);
            let candidate = AttachmentSurface::from_snapshot(&snapshot).unwrap();
            present_surface_with_writer(
                &mut output,
                &candidate,
                &mut presenter,
                &viewport,
                &status,
                TerminalViewTransportState::Synchronizing,
            )
            .unwrap();
            surface = candidate;
            viewport.observe_presentation();

            // Same resume completion and presentation path as the Active event.
            let resumed = viewport.finish_resume().is_some();
            assert!(resumed && viewport.is_live());
            let active_painted = present_transport_transition_with_writer(
                &mut output,
                &surface,
                &mut presenter,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
                resumed,
            )
            .unwrap();
            let expected = ComposedFrame::compose(
                &surface.surface,
                presenter.baseline.as_ref(),
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .unwrap();
            let before_click = presenter.baseline.as_ref().unwrap().rows == expected.rows;
            let kept_history = (0..layout.child.rows).all(|row| {
                let width = usize::from(layout.child.columns);
                presenter.baseline.as_ref().unwrap().rows[&row][..width]
                    == history_rows[&row][..width]
            });

            // A later pointer-driven presentation repairs the stale content.
            present_surface_with_writer(
                &mut output,
                &surface,
                &mut presenter,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .unwrap();
            let after_click = presenter.baseline.as_ref().unwrap().rows == expected.rows;
            eprintln!(
                "route={route:?} new_output={new_output} live_before_click={before_click} kept_history={kept_history} active_painted={active_painted} live_after_click={after_click}"
            );
            assert!(
                after_click,
                "the later presentation must demonstrate the reported repair"
            );
            if !before_click {
                missing.push((route, new_output));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "resume left old history visible in {missing:?}"
    );
}
