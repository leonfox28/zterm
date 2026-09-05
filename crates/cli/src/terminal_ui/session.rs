use super::*;
use zterm_daemon::client::view::{TerminalViewCommandWriter, TerminalViewEventReader};

/// One owner for attachment semantics, presentation, navigation, and input fences.
/// The presenter commits physical output; surface and viewport retain their own
/// validated semantic and history states at that successful presentation boundary.
pub(super) struct TerminalUiSession {
    pub(super) session_id: SessionId,
    pub(super) events: TerminalViewEventReader,
    pub(super) writer: TerminalViewCommandWriter,
    pub(super) input_epoch: InputEpoch,
    pub(super) current_input_epoch: u64,
    pub(super) stdin_pump: StdinPump,
    pub(super) prefix: PrefixParser,
    pub(super) transport_state: TerminalViewTransportState,
    pub(super) resize_coalescer: ResizeCoalescer,
    pub(super) physical_size: TerminalSize,
    pub(super) surface: AttachmentSurface,
    pub(super) presenter: DesktopPresenter,
    pub(super) selection: SelectionController,
    pub(super) viewport: ViewportController,
    pub(super) status_renderer: StatusRenderer,
    pub(super) viewport_pacer: ViewportPresentationPacer,
    pub(super) sync_requested: bool,
    pub(super) input_codec: HostInputCodec,
    pub(super) copy_key_lease: CopyKeyLease,
    pub(super) deferred_active: bool,
}

impl TerminalUiSession {
    pub(super) async fn run(
        mut self,
        stdin: &io::Stdin,
        stdout: &io::Stdout,
        resize_signal: &mut Signal,
        cancellation_receiver: &mut watch::Receiver<Option<TerminalSignalCancellation>>,
    ) -> Result<TerminalCompletion, CliError> {
        let result = self
            .run_loop(stdin, stdout, resize_signal, cancellation_receiver)
            .await;
        finish_terminal_view(result, &mut self.stdin_pump, &self.writer).await
    }

    async fn run_loop(
        &mut self,
        stdin: &io::Stdin,
        stdout: &io::Stdout,
        resize_signal: &mut Signal,
        cancellation_receiver: &mut watch::Receiver<Option<TerminalSignalCancellation>>,
    ) -> Result<TerminalCompletion, CliError> {
        'terminal: loop {
            let now = Instant::now();
            if self.viewport_pacer.due(now) {
                if let Err(error) = present_cached_viewport_stdout(
                    &self.surface,
                    &mut self.presenter,
                    &mut self.viewport,
                    &self.status_renderer,
                    self.transport_state,
                    &mut self.viewport_pacer,
                    CachedPresentationRequest { now, force: false },
                ) {
                    break Err(error);
                }
                continue;
            }
            if self
                .prefix
                .deadline()
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                self.flush_prefix().await?;
                continue;
            }
            let prefix_deadline = self.prefix.deadline();
            let viewport_deadline = self.viewport_pacer.deadline();
            tokio::select! {
                cancellation = receive_terminal_cancellation(cancellation_receiver) => {
                    self.viewport_pacer.cancel();
                    break Err(cancellation.error(Some(self.session_id)));
                }
                signal = resize_signal.recv() => {
                    if signal.is_none() {
                        break Err(terminal_daemon_error(
                            DomainErrorKind::Cancelled,
                            "SIGWINCH handler closed",
                        ));
                    }
                    let latest_physical = match terminal_size(stdout) {
                        Ok(size) => size,
                        Err(error) => break Err(error),
                    };
                    self.physical_size = latest_physical;
                    let layout = ChromeLayout::new(
                        latest_physical,
                        self.viewport.effective_screen(self.surface.active_screen()),
                    );
                    let latest = layout.child;
                    self.viewport_pacer.cancel();
                    self.viewport.set_layout(layout);
                    self.status_renderer.resize(latest_physical);
                    self.selection.cancel();
                    reconcile_presenter_selection(
                        &mut self.selection,
                        &self.viewport,
                        &self.surface,
                        &mut self.presenter,
                    );
                    if let Err(error) = present_surface_stdout(
                        &self.surface,
                        &mut self.presenter,
                        &self.viewport,
                        &self.status_renderer,
                        self.transport_state,
                    ) {
                        break Err(error);
                    }
                    self.viewport.observe_presentation();
                    self.viewport_pacer.mark_presented(Instant::now());
                    if let Some(size) = self.resize_coalescer.observe(latest, self.transport_state) {
                        if let Err(error) = self.writer.resize(size).await {
                            break Err(error.into());
                        }
                        if let Err(error) = self.transition_transport(stdin, TerminalViewTransportState::Synchronizing).await {
                            break 'terminal Err(error);
                        }
                    }
                }
                // The explicit expired-deadline check above makes this local
                // timeout independent of continuously ready terminal output.
                () = wait_for_prefix_deadline(prefix_deadline), if prefix_deadline.is_some() => {
                    self.flush_prefix().await?;
                }
                () = wait_for_viewport_deadline(viewport_deadline), if viewport_deadline.is_some() => {
                    let now = Instant::now();
                    if let Err(error) = present_cached_viewport_stdout(
                        &self.surface,
                        &mut self.presenter,
                        &mut self.viewport,
                        &self.status_renderer,
                        self.transport_state,
                        &mut self.viewport_pacer,
                        CachedPresentationRequest { now, force: false },
                    ) {
                        break Err(error);
                    }
                }
                event = self.events.read_event() => {
                    let event = match event {
                        Ok(Some(event)) => event,
                        Ok(None) => {
                            break Err(terminal_daemon_error(
                                DomainErrorKind::Cancelled,
                                "terminal attachment event stream closed",
                            ));
                        }
                        Err(error) => break Err(error.into()),
                    };
                    match self.handle_event(stdin, event).await {
                        Ok(Some(completion)) => break Ok(completion),
                        Ok(None) => {},
                        Err(error) => break Err(error),
                    }
                }
                input = self.stdin_pump.recv() => {
                    match input {
                        Some(StdinEvent::Bytes { epoch, bytes })
                            if input_epoch_is_current(epoch, self.current_input_epoch) =>
                        {
                            // A paced history frame may have committed a new
                            // source since the preceding pointer event. Retire
                            // any old coordinates before interpreting a copy
                            // key against the physical keyboard mode.
                            reconcile_presenter_selection(
                                &mut self.selection,
                                &self.viewport,
                                &self.surface,
                                &mut self.presenter,
                            );
                            let mut host_events = match self.input_codec.feed(&bytes) {
                                Ok(events) => VecDeque::from(events),
                                Err(error) => break 'terminal Err(error),
                            };
                            let mut force_viewport_presentation = false;
                            while let Some(host_event) = host_events.pop_front() {
                                match host_event {
                                    HostInputEvent::Bytes(bytes) => {
                                        if let Err(error) = invalidate_selection_stdout(
                                            &mut self.selection,
                                            &mut self.viewport,
                                            &self.surface,
                                            &mut self.presenter,
                                            &self.status_renderer,
                                            self.transport_state,
                                            &mut self.viewport_pacer,
                                        ) {
                                            break 'terminal Err(error);
                                        }
                                        for action in self.prefix.feed(&bytes, Instant::now()) {
                                            match action {
                                                PrefixAction::Input(bytes) if self.viewport.is_live()
                                                    && self.transport_state
                                                        == TerminalViewTransportState::Active =>
                                                {
                                                    if let Err(error) = self.writer.write_input(bytes).await {
                                                        break 'terminal Err(error.into());
                                                    }
                                                }
                                                PrefixAction::Input(bytes) if !self.viewport.is_live() => {
                                                    let effect = self.viewport.retain_or_resume(bytes)?;
                                                    self.apply_viewport(effect, true).await?;
                                                }
                                                PrefixAction::Input(_) => {}
                                                PrefixAction::Detach => break,
                                            }
                                        }
                                    }
                                    HostInputEvent::LegacyCtrlC => {
                                        if self.selection.is_finalized() {
                                            if let Err(error) = write_selection_clipboard_stdout(
                                                &self.selection,
                                                &self.viewport,
                                                &self.surface,
                                                &mut self.presenter,
                                            ) {
                                                break 'terminal Err(error);
                                            }
                                        } else {
                                            host_events
                                                .push_front(HostInputEvent::Bytes(vec![0x03]));
                                        }
                                    }
                                    HostInputEvent::EnhancedKey(key) => {
                                        let outer_flags = self.presenter.presented_keyboard_flags();
                                        match route_enhanced_input(
                                            &key,
                                            self.surface.modes(),
                                            outer_flags,
                                            self.selection.is_finalized(),
                                            &mut self.copy_key_lease,
                                        ) {
                                            KeyboardRoute::Copy => {
                                                if let Err(error) =
                                                    write_selection_clipboard_stdout(
                                                        &self.selection,
                                                        &self.viewport,
                                                        &self.surface,
                                                        &mut self.presenter,
                                                    )
                                                {
                                                    break 'terminal Err(error);
                                                }
                                            }
                                            KeyboardRoute::Consume => {}
                                            KeyboardRoute::Forward {
                                                bytes,
                                                clear_selection,
                                                reinterpret_legacy,
                                            } => {
                                                if clear_selection
                                                    && let Err(error) =
                                                        invalidate_selection_stdout(
                                                            &mut self.selection,
                                                            &mut self.viewport,
                                                            &self.surface,
                                                            &mut self.presenter,
                                                            &self.status_renderer,
                                                            self.transport_state,
                                                            &mut self.viewport_pacer,
                                                        )
                                                {
                                                    break 'terminal Err(error);
                                                }
                                                let forwarded = if reinterpret_legacy {
                                                    host_events_from_legacy_bytes(bytes)
                                                } else if bytes.is_empty() {
                                                    Vec::new()
                                                } else {
                                                    vec![HostInputEvent::Bytes(bytes)]
                                                };
                                                for event in forwarded.into_iter().rev() {
                                                    host_events.push_front(event);
                                                }
                                            }
                                        }
                                    }
                                    HostInputEvent::Paste(bytes) => {
                                        if let Err(error) = invalidate_selection_stdout(
                                            &mut self.selection,
                                            &mut self.viewport,
                                            &self.surface,
                                            &mut self.presenter,
                                            &self.status_renderer,
                                            self.transport_state,
                                            &mut self.viewport_pacer,
                                        ) {
                                            break 'terminal Err(error);
                                        }
                                        if self.viewport.is_live()
                                            && self.transport_state
                                                == TerminalViewTransportState::Active
                                        {
                                            if let Err(error) = self.writer.write_input(bytes).await {
                                                break 'terminal Err(error.into());
                                            }
                                        } else if !self.viewport.is_live() {
                                            let effect = self.viewport.retain_or_resume(bytes)?;
                                            self.apply_viewport(effect, true).await?;
                                        }
                                    }
                                    HostInputEvent::PageUp | HostInputEvent::PageDown => {
                                        if let Err(error) = invalidate_selection_stdout(
                                            &mut self.selection,
                                            &mut self.viewport,
                                            &self.surface,
                                            &mut self.presenter,
                                            &self.status_renderer,
                                            self.transport_state,
                                            &mut self.viewport_pacer,
                                        ) {
                                            break 'terminal Err(error);
                                        }
                                        let older = matches!(host_event, HostInputEvent::PageUp);
                                        let raw = if older { PAGE_UP } else { PAGE_DOWN };
                                        if self.viewport.is_resume_pending() {
                                            self.viewport.retain_resume_input(raw)?;
                                        } else if self.viewport.is_history()
                                            || live_history_navigation_allowed(self.transport_state)
                                                && history_owns_gestures(
                                                    self.surface.active_screen(),
                                                    self.surface.modes(),
                                                )
                                        {
                                            let effect = self.viewport.navigate(
                                                older,
                                                usize::from(self.viewport.content_size().rows)
                                                    .saturating_sub(1)
                                                    .max(1),
                                            );
                                            self.apply_viewport(effect, true).await?;
                                        } else if self.transport_state
                                            == TerminalViewTransportState::Active
                                            && let Err(error) = self.writer.write_input(raw.to_vec()).await
                                        {
                                            break 'terminal Err(error.into());
                                        }
                                    }
                                    HostInputEvent::Mouse(mouse) => {
                                        let routed = match route_pointer(
                                            &mouse,
                                            &mut self.viewport,
                                            &self.surface,
                                            &mut self.selection,
                                            live_history_navigation_allowed(self.transport_state),
                                        ) {
                                            Ok(routed) => routed,
                                            Err(error) => break 'terminal Err(error),
                                        };
                                        reconcile_presenter_selection(
                                            &mut self.selection,
                                            &self.viewport,
                                            &self.surface,
                                            &mut self.presenter,
                                        );
                                        match routed {
                                            PointerRoute::Viewport(effect) => {
                                                force_viewport_presentation |= mouse.release;
                                                self.apply_viewport(effect, true).await?;
                                            }
                                            PointerRoute::Child(bytes)
                                                if self.viewport.is_resume_pending() =>
                                            {
                                                self.viewport.retain_resume_input(&bytes)?;
                                            }
                                            PointerRoute::Child(bytes)
                                                if self.viewport.is_live()
                                                    && self.transport_state
                                                        == TerminalViewTransportState::Active =>
                                            {
                                                if let Err(error) = self.writer.write_input(bytes).await {
                                                    break 'terminal Err(error.into());
                                                }
                                            }
                                            PointerRoute::Child(_) | PointerRoute::Ignore => {}
                                            PointerRoute::SelectionChanged => {
                                                let now = Instant::now();
                                                if mark_cached_viewport_dirty(
                                                    &self.viewport,
                                                    &mut self.viewport_pacer,
                                                    now,
                                                ) {
                                                    force_viewport_presentation |= mouse.release;
                                                }
                                            }
                                        }
                                    }
                                }
                                if self.prefix.detached() {
                                    break;
                                }
                            }
                            if self.prefix.detached() {
                                self.viewport_pacer.cancel();
                                break Ok(TerminalCompletion::Detached);
                            }
                            if self.deferred_active && !self.input_codec.paste_in_progress() {
                                if let Err(error) = self.transition_transport(stdin, TerminalViewTransportState::Active).await {
                                    break 'terminal Err(error);
                                }
                                self.deferred_active = false;
                            }
                            if let Err(error) = present_cached_viewport_stdout(
                                &self.surface,
                                &mut self.presenter,
                                &mut self.viewport,
                                &self.status_renderer,
                                self.transport_state,
                                &mut self.viewport_pacer,
                                CachedPresentationRequest {
                                    now: Instant::now(),
                                    force: force_viewport_presentation,
                                },
                            ) {
                                break 'terminal Err(error);
                            }
                        }
                        Some(StdinEvent::Bytes { .. }) => {}
                        Some(StdinEvent::Eof) | None => {
                            self.viewport_pacer.cancel();
                            if let Some(bytes) = take_pending_active_input(
                                &mut self.prefix,
                                self.transport_state,
                            ) && self.viewport.is_live()
                                && let Err(error) = self.writer.write_input(bytes).await
                            {
                                break Err(error.into());
                            }
                            break Ok(TerminalCompletion::Detached);
                        }
                        Some(StdinEvent::Error(detail)) => {
                            break Err(CliError::Io(format!("read terminal stdin: {detail}")));
                        }
                    }
                }
            }
        }
    }

    async fn handle_event(
        &mut self,
        stdin: &impl AsFd,
        event: TerminalViewEvent,
    ) -> Result<Option<TerminalCompletion>, CliError> {
        match event {
            TerminalViewEvent::TransportState(state) => {
                if should_defer_active_for_paste(state, &self.viewport, &self.input_codec) {
                    self.deferred_active = true;
                    return Ok(None);
                }
                self.deferred_active = false;
                self.transition_transport(stdin, state).await?;
            }
            TerminalViewEvent::ConnectionStatus(status) => {
                self.status_renderer.observe(status)?;
                present_surface_stdout(
                    &self.surface,
                    &mut self.presenter,
                    &self.viewport,
                    &self.status_renderer,
                    self.transport_state,
                )?;
                self.viewport.observe_presentation();
                self.viewport_pacer.mark_presented(Instant::now());
            }
            TerminalViewEvent::Snapshot(snapshot) => {
                self.viewport_pacer.cancel();
                self.selection.cancel();
                reconcile_presenter_selection(
                    &mut self.selection,
                    &self.viewport,
                    &self.surface,
                    &mut self.presenter,
                );
                let preserving_resume_input = self.viewport.is_resume_pending();
                if self.transport_state != TerminalViewTransportState::Synchronizing
                    && !preserving_resume_input
                {
                    transition_input_state(
                        stdin,
                        &self.input_epoch,
                        &mut self.current_input_epoch,
                        &mut self.stdin_pump,
                        &mut self.prefix,
                        TerminalViewTransportState::Synchronizing,
                    )?;
                    self.transport_state = TerminalViewTransportState::Synchronizing;
                }
                self.viewport
                    .observe_snapshot(snapshot.surface.scroll_metrics);
                let layout = ChromeLayout::new(
                    self.physical_size,
                    self.viewport
                        .effective_screen(snapshot.surface.active_screen),
                );
                self.viewport.set_layout(layout);
                let _ = self
                    .resize_coalescer
                    .observe(layout.child, self.transport_state);
                let history_refill = self.viewport.refetch_history_window();
                let rendered = install_snapshot_stdout(
                    &mut self.surface,
                    &mut self.presenter,
                    &snapshot,
                    &self.viewport,
                    &self.status_renderer,
                    self.transport_state,
                );
                rendered?;
                self.viewport.observe_presentation();
                self.viewport_pacer.mark_presented(Instant::now());
                self.prefix.clear_pending();
                self.sync_requested = false;
                self.writer.revision_applied(snapshot.revision);
                self.writer.snapshot_applied(snapshot.revision).await?;
                if let Some(query) = history_refill
                    && let Err(error) = self.writer.request_history_window(query).await
                {
                    return Err(error.into());
                }
            }
            TerminalViewEvent::Delta(delta) => {
                // A delta may itself change Main/Alternate layout and submit a
                // resize below. That resize starts a *new* snapshot epoch, so the
                // old delta is an activation barrier only when this view was already
                // synchronizing as the event entered the handler.
                let acknowledges_existing_sync =
                    delta_acknowledges_existing_sync(self.transport_state);
                let rendered_live = self.viewport.is_live();
                if rendered_live {
                    self.viewport_pacer.cancel();
                }
                let delta_result = apply_delta_stdout(
                    &mut self.surface,
                    &mut self.presenter,
                    &delta,
                    &mut self.viewport,
                    &mut self.selection,
                    &self.status_renderer,
                    self.transport_state,
                );
                match delta_result {
                    Ok(DeltaRender::Applied) => {
                        self.sync_requested = false;
                        self.writer.revision_applied(delta.to_revision);
                        if rendered_live {
                            self.viewport.observe_presentation();
                            self.viewport_pacer.mark_presented(Instant::now());
                            let mode_resize = self
                                .resize_coalescer
                                .observe(self.viewport.content_size(), self.transport_state);
                            if let Some(size) = mode_resize {
                                self.writer.resize(size).await?;
                                self.transition_transport(
                                    stdin,
                                    TerminalViewTransportState::Synchronizing,
                                )
                                .await?;
                            }
                        } else {
                            cancel_unpresentable_cached_viewport(
                                &self.viewport,
                                &mut self.viewport_pacer,
                            );
                        }
                        if acknowledges_existing_sync
                            && let Err(error) =
                                self.writer.snapshot_applied(delta.to_revision).await
                        {
                            return Err(error.into());
                        }
                    }
                    Ok(DeltaRender::Gap) => {
                        self.viewport_pacer.cancel();
                        self.selection.cancel();
                        reconcile_presenter_selection(
                            &mut self.selection,
                            &self.viewport,
                            &self.surface,
                            &mut self.presenter,
                        );
                        if rendered_live {
                            self.viewport.begin_resume(Vec::new())?;
                        }
                        if self.transport_state != TerminalViewTransportState::Synchronizing
                            && !self.viewport.is_resume_pending()
                        {
                            transition_input_state(
                                stdin,
                                &self.input_epoch,
                                &mut self.current_input_epoch,
                                &mut self.stdin_pump,
                                &mut self.prefix,
                                TerminalViewTransportState::Synchronizing,
                            )?;
                            self.transport_state = TerminalViewTransportState::Synchronizing;
                        }
                        if !self.sync_requested {
                            self.sync_requested = true;
                            self.writer.request_sync(self.surface.revision()).await?;
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            TerminalViewEvent::HistoryWindow(result) => {
                let effect = self.viewport.apply_view_history_window(result)?;
                reconcile_presenter_selection_for_next_frame(
                    &mut self.selection,
                    &self.viewport,
                    &self.surface,
                    &mut self.presenter,
                );
                self.apply_viewport(effect, false).await?;
                reconcile_presenter_selection(
                    &mut self.selection,
                    &self.viewport,
                    &self.surface,
                    &mut self.presenter,
                );
            }
            TerminalViewEvent::ClipboardWrite(write) => {
                let stdout = io::stdout();
                let mut output = stdout.lock();
                self.presenter.write_clipboard(&mut output, &write)?;
            }
            TerminalViewEvent::SyncRequired { .. } => {
                self.viewport_pacer.cancel();
                self.selection.cancel();
                reconcile_presenter_selection(
                    &mut self.selection,
                    &self.viewport,
                    &self.surface,
                    &mut self.presenter,
                );
                self.viewport.observe_sync_required();
                if self.transport_state != TerminalViewTransportState::Synchronizing {
                    self.transport_state = TerminalViewTransportState::Synchronizing;
                }
                // The marker and its authoritative replacement snapshot are emitted
                // together. Keep the last complete host presentation untouched while
                // that replacement is in flight instead of repainting an identical
                // history frame or clearing attachment scroll state with a redundant
                // sync request.
                self.sync_requested = true;
            }
            TerminalViewEvent::LeaseLost { .. } => {
                self.viewport_pacer.cancel();
                return Err(terminal_daemon_error(
                    DomainErrorKind::LeaseLost,
                    "another attachment took over this terminal controller",
                ));
            }
            TerminalViewEvent::SessionEnded(ended) => {
                self.viewport_pacer.cancel();
                return terminal_end_completion(ended.reason).map(Some);
            }
        }
        Ok(None)
    }

    async fn flush_prefix(&mut self) -> Result<(), CliError> {
        if let Some(bytes) = self.prefix.flush_pending() {
            if self.viewport.is_live() && self.transport_state == TerminalViewTransportState::Active
            {
                self.writer.write_input(bytes).await?;
            } else if !self.viewport.is_live() {
                let effect = self.viewport.retain_or_resume(bytes)?;
                self.apply_viewport(effect, false).await?;
            }
        }
        Ok(())
    }

    async fn apply_viewport(
        &mut self,
        effect: ViewportEffect,
        force: bool,
    ) -> Result<(), CliError> {
        if apply_viewport_effect(
            effect,
            &mut self.viewport,
            &self.surface,
            &mut self.presenter,
            &self.writer,
            &self.status_renderer,
            self.transport_state,
            &mut self.viewport_pacer,
            force,
        )
        .await?
        {
            self.sync_requested = true;
        }
        Ok(())
    }

    async fn transition_transport(
        &mut self,
        stdin: &impl AsFd,
        next: TerminalViewTransportState,
    ) -> Result<(), CliError> {
        let previous = self.transport_state;
        self.viewport_pacer.cancel();
        let (next, pending_resize) = self.resize_coalescer.enter_transport_state(next);
        if next == TerminalViewTransportState::Reconnecting {
            self.viewport.reset_presentation_for_reconnect();
            self.status_renderer.reset_for_reconnect();
        }
        if next != previous && next != TerminalViewTransportState::Active {
            self.selection.cancel();
        }
        let resume_input = transition_transport_input_state(
            stdin,
            &self.input_epoch,
            &mut self.current_input_epoch,
            &mut self.stdin_pump,
            &mut self.prefix,
            previous,
            next,
            &mut self.viewport,
        )?;
        reconcile_presenter_selection(
            &mut self.selection,
            &self.viewport,
            &self.surface,
            &mut self.presenter,
        );
        if present_transport_transition_stdout(
            &self.surface,
            &mut self.presenter,
            &self.viewport,
            &self.status_renderer,
            next,
            resume_input.is_some(),
        )? {
            self.viewport.observe_presentation();
            self.viewport_pacer.mark_presented(Instant::now());
        }
        if let Some(size) = pending_resize {
            self.writer.resize(size).await?;
        }
        if let Some(bytes) = resume_input
            && !bytes.is_empty()
        {
            self.writer.write_input(bytes).await?;
        }
        self.transport_state = next;
        Ok(())
    }
}
