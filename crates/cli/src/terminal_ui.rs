//! Unix raw-terminal ownership and semantic terminal presentation.

use super::{CliError, TerminalRequest};
use zterm_daemon::operations::LocalRuntime;

/// Runs one deferred terminal request after validating the physical terminal.
pub async fn run_terminal(
    request: TerminalRequest,
    runtime: &LocalRuntime,
) -> Result<(), CliError> {
    #[cfg(unix)]
    {
        unix::run(request, runtime).await
    }
    #[cfg(not(unix))]
    {
        let _ = (request, runtime);
        Err(CliError::Daemon(zterm_daemon::error::DaemonError::new(
            zterm_core::DomainErrorKind::UnsupportedPlatform,
            "interactive terminal attachment is not supported on this platform",
        )))
    }
}

#[cfg(unix)]
mod unix {
    use std::collections::VecDeque;
    use std::fmt;
    use std::future::Future;
    use std::io::{self, IsTerminal, Write};
    use std::os::fd::{AsFd, OwnedFd};
    use std::panic::AssertUnwindSafe;
    use std::sync::Arc;
    #[cfg(test)]
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[cfg(test)]
    use std::sync::mpsc::{Receiver as TestReceiver, SyncSender as TestSender, sync_channel};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use futures_util::FutureExt;
    use nix::sys::termios::{FlushArg, SetArg, Termios, cfmakeraw, tcflush, tcgetattr, tcsetattr};
    use rustix::event::{PollFd, PollFlags, poll};
    use tokio::signal::unix::{Signal, SignalKind, signal};
    use tokio::sync::{mpsc, watch};
    #[cfg(test)]
    use zterm_core::Revision;
    use zterm_core::terminal::{
        ActiveScreen, TerminalHistoryWindowAnchor, TerminalHistoryWindowQuery, TerminalModes,
        TerminalMouseEncoding, TerminalMouseMode, TerminalScrollMetrics, TerminalSize,
        TerminalSurfaceDelta, TerminalSurfaceHistoryWindowResult, TerminalSurfaceRow,
        TerminalSurfaceSnapshot, TerminalViewportDisposition,
    };
    #[cfg(test)]
    use zterm_core::terminal::{
        TerminalCell, TerminalColor, TerminalCursor, TerminalStyle, TerminalSurface,
        TerminalSurfaceHistoryWindowFrame, TerminalSurfaceRowPatch,
    };
    use zterm_core::terminal_selection::{TerminalTextPoint, TerminalTextSelectionError};
    use zterm_core::viewport_cache::{
        CachedViewportWindow, ViewportAnchorObservation, ViewportCache, ViewportCacheUpdate,
    };
    use zterm_core::{DomainErrorKind, RESERVED_DEVICE_ALIAS, SessionId};
    use zterm_daemon::error::DaemonError;
    use zterm_daemon::operations::{
        LocalRuntime, PreparedTerminalView, TerminalViewConnectionPath,
        TerminalViewConnectionStatus, TerminalViewEndReason, TerminalViewEvent,
        TerminalViewHistoryWindow, TerminalViewTransportState,
    };

    use super::super::{CliError, TerminalRequest, TerminalRequestKind};

    mod attachment_surface {
        include!("terminal_ui/surface.rs");
    }
    mod ansi_presenter {
        include!("terminal_ui/ansi_presenter.rs");
    }
    mod composition {
        include!("terminal_ui/composition.rs");
    }
    mod keyboard {
        include!("terminal_ui/keyboard.rs");
    }
    mod selection {
        include!("terminal_ui/selection.rs");
    }

    use ansi_presenter::DesktopPresenter;
    #[cfg(test)]
    use ansi_presenter::semantic_dirty_runs;
    use attachment_surface::AttachmentSurface;
    use composition::{
        ChromeLayout, ComposedFrame, LiveViewportProjection, ScrollbarGeometry,
        normalize_composed_row,
    };
    use keyboard::{CopyKeyLease, EnhancedKey, KeyEventKind};
    use selection::{SelectionController, SelectionSourceIdentity};

    const STDIN_CHANNEL_CAPACITY: usize = 8;
    const STDIN_CHUNK_BYTES: usize = 4 * 1024;
    const CONTROL_PREFIX_TIMEOUT: Duration = Duration::from_secs(1);
    const DETACH_TIMEOUT: Duration = Duration::from_secs(2);
    const DRAG_REQUEST_INTERVAL: Duration = Duration::from_millis(33);
    const MIN_VIEWPORT_PRESENT_INTERVAL: Duration = Duration::from_millis(16);
    const HOST_SYNC_BEGIN: &[u8] = b"\x1b[?2026h";
    const HOST_SYNC_END: &[u8] = b"\x1b[?2026l";
    const HOST_INPUT_CAPTURE: &[u8] = b"\x1b[?1003h\x1b[?1006h";
    const ENTER_TERMINAL_UI: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[>0u\x1b[?1003h\x1b[?1006h";
    const HOST_SEQUENCE_BOUND: usize = 64;
    const RESUME_INPUT_BOUND: usize = 1024 * 1024 - 1024;
    const PAGE_UP: &[u8] = b"\x1b[5~";
    const PAGE_DOWN: &[u8] = b"\x1b[6~";
    const PASTE_START: &[u8] = b"\x1b[200~";
    const PASTE_END: &[u8] = b"\x1b[201~";
    const RESTORE_TERMINAL_UI: &[u8] = concat!(
        "\x1b[?2026l",
        "\x1b[?9l",
        "\x1b[?1000l",
        "\x1b[?1001l",
        "\x1b[?1002l",
        "\x1b[?1003l",
        "\x1b[?1004l",
        "\x1b[?1005l",
        "\x1b[?1006l",
        "\x1b[?1007l",
        "\x1b[?1015l",
        "\x1b[?1016l",
        "\x1b[?2004l",
        "\x1b[?1l",
        "\x1b>",
        "\x1b[<u",
        "\x1b[0m",
        "\x1b[?25h",
        "\x1b[?1049l"
    )
    .as_bytes();

    pub(super) async fn run(
        request: TerminalRequest,
        runtime: &LocalRuntime,
    ) -> Result<(), CliError> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        if !stdin.is_terminal() || !stdout.is_terminal() {
            return Err(CliError::Usage(
                "interactive attachment requires both stdin and stdout to be TTYs".to_owned(),
            ));
        }

        let panic_hook = ScopedPanicHook::suppress();
        let result = AssertUnwindSafe(run_guarded_terminal(request, runtime))
            .catch_unwind()
            .await;
        drop(panic_hook);
        match result {
            Ok(Ok(completion)) => emit_completion_diagnostic(completion),
            Ok(Err(error)) => Err(error),
            Err(payload) => {
                drop(payload);
                Err(CliError::Io(
                    "interactive terminal aborted after an internal panic".to_owned(),
                ))
            }
        }
    }

    async fn run_guarded_terminal(
        request: TerminalRequest,
        runtime: &LocalRuntime,
    ) -> Result<TerminalCompletion, CliError> {
        io::stdout()
            .flush()
            .map_err(|error| terminal_io("flush stdout before raw mode", error))?;
        let TerminalSignals {
            mut resize,
            mut interrupt,
            mut terminate,
            mut hangup,
        } = TerminalSignals::install()?;
        let (cancellation_sender, mut cancellation_receiver) = watch::channel(None);
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut guard = TerminalGuard::enter(&stdin, &stdout)?;
        let result = select_terminal_termination(
            run_view(
                request,
                runtime,
                &stdin,
                &stdout,
                &mut resize,
                &mut cancellation_receiver,
            ),
            &mut interrupt,
            &mut terminate,
            &mut hangup,
            cancellation_sender,
        )
        .await;
        let restoration = guard.restore();
        match (result, restoration) {
            (_, Err(error)) => Err(error),
            (Err(error), Ok(())) => Err(error),
            (Ok(completion), Ok(())) => Ok(completion),
        }
    }

    async fn select_terminal_termination(
        view: impl Future<Output = Result<TerminalCompletion, CliError>>,
        interrupt: &mut Signal,
        terminate: &mut Signal,
        hangup: &mut Signal,
        cancellation_sender: watch::Sender<Option<TerminalSignalCancellation>>,
    ) -> Result<TerminalCompletion, CliError> {
        tokio::pin!(view);
        let cancellation = tokio::select! {
            biased;
            signal = interrupt.recv() => TerminalSignalCancellation::new("SIGINT", signal),
            signal = terminate.recv() => TerminalSignalCancellation::new("SIGTERM", signal),
            signal = hangup.recv() => TerminalSignalCancellation::new("SIGHUP", signal),
            result = &mut view => return result,
        };
        let _ = cancellation_sender.send(Some(cancellation));
        view.await
    }

    async fn run_view(
        request: TerminalRequest,
        runtime: &LocalRuntime,
        stdin: &io::Stdin,
        stdout: &io::Stdout,
        resize_signal: &mut Signal,
        cancellation_receiver: &mut watch::Receiver<Option<TerminalSignalCancellation>>,
    ) -> Result<TerminalCompletion, CliError> {
        if let Some(cancellation) = current_terminal_cancellation(cancellation_receiver) {
            return Err(cancellation.error(None));
        }
        let initial_physical_size = terminal_size(stdout)?;
        let remote_request = terminal_request_is_remote(&request);
        let initial_layout =
            ChromeLayout::new(initial_physical_size, remote_request, ActiveScreen::Main);
        let initial_size = initial_layout.child;
        let escape = request.escape;
        let stateful_prepare = matches!(
            &request.kind,
            TerminalRequestKind::Create { .. }
                | TerminalRequestKind::Attach {
                    create_main: true,
                    ..
                }
        );
        let input_epoch = InputEpoch::new();
        let mut current_input_epoch = input_epoch.current();
        let mut stdin_pump = StdinPump::start(stdin, input_epoch.clone())?;
        let mut prefix = PrefixParser::new(escape.0);
        let mut transport_state = TerminalViewTransportState::Synchronizing;
        let mut resize_coalescer = ResizeCoalescer::new(initial_size);
        let prepared = match await_while_inactive(
            prepare(request, runtime, initial_size),
            InactiveWaitContext {
                stdout,
                resize_signal,
                cancellation_receiver,
                stdin_pump: &mut stdin_pump,
                prefix: &mut prefix,
                resize_coalescer: &mut resize_coalescer,
                current_input_epoch,
                preserve_submitted_result: stateful_prepare,
                remote: remote_request,
            },
        )
        .await?
        {
            InactiveWait::Ready(prepared) => prepared,
            InactiveWait::Cancelled(cancellation) => {
                stdin_pump.shutdown()?;
                return inactive_cancellation_result(cancellation, None);
            }
            InactiveWait::CompletedAfterCancellation {
                value: prepared,
                cancellation,
            } => {
                let session_id = prepared.session_id();
                drop(prepared);
                stdin_pump.shutdown()?;
                return inactive_cancellation_result(cancellation, Some(session_id));
            }
        };
        let session_id = prepared.session_id();
        let mut physical_size = terminal_size(stdout)?;
        let latest_layout = ChromeLayout::new(
            physical_size,
            remote_request,
            prepared.initial_snapshot().surface.active_screen,
        );
        let latest_size = latest_layout.child;
        if latest_size != initial_size || resize_coalescer.pending.is_some() {
            let _ = resize_coalescer.observe(latest_size, transport_state);
        }

        let remote_alias = prepared.remote_alias().map(str::to_owned);
        if remote_alias.is_some() != remote_request {
            drop(prepared);
            stdin_pump.shutdown()?;
            return Err(terminal_daemon_error(
                DomainErrorKind::MalformedFrame,
                "resolved terminal target changed local/remote class",
            ));
        }

        let mut surface = AttachmentSurface::from_snapshot(prepared.initial_snapshot())?;
        let mut presenter = DesktopPresenter::default();
        let mut selection = SelectionController::default();
        let initial_scroll_metrics = prepared.initial_snapshot().surface.scroll_metrics;
        let mut viewport = ViewportController::with_layout(latest_layout, initial_scroll_metrics);
        let mut status_renderer = StatusRenderer::new(remote_alias, physical_size);
        reconcile_presenter_selection(&mut selection, &viewport, &surface, &mut presenter);
        present_surface_stdout(
            &surface,
            &mut presenter,
            &viewport,
            &status_renderer,
            transport_state,
        )?;
        viewport.observe_presentation();
        let mut viewport_pacer = ViewportPresentationPacer::default();
        viewport_pacer.mark_presented(Instant::now());
        if let Some(cancellation) = current_terminal_cancellation(cancellation_receiver) {
            drop(prepared);
            stdin_pump.shutdown()?;
            return inactive_cancellation_result(
                InactiveCancellation::Signal(cancellation),
                Some(session_id),
            );
        }
        let view = match await_while_inactive(
            async move { prepared.acknowledge_initial().await.map_err(Into::into) },
            InactiveWaitContext {
                stdout,
                resize_signal,
                cancellation_receiver,
                stdin_pump: &mut stdin_pump,
                prefix: &mut prefix,
                resize_coalescer: &mut resize_coalescer,
                current_input_epoch,
                preserve_submitted_result: false,
                remote: remote_request,
            },
        )
        .await?
        {
            InactiveWait::Ready(view) => view,
            InactiveWait::Cancelled(cancellation) => {
                stdin_pump.shutdown()?;
                return inactive_cancellation_result(cancellation, Some(session_id));
            }
            InactiveWait::CompletedAfterCancellation {
                value: view,
                cancellation,
            } => {
                drop(view);
                stdin_pump.shutdown()?;
                return inactive_cancellation_result(cancellation, Some(session_id));
            }
        };
        let (mut events, writer) = view.split();
        if let Some(query) = viewport.prefetch_live()
            && writer.request_history_window(query).await.is_err()
        {
            // Live prefetch is opportunistic. Preserve the valid attachment
            // and let the first real gesture retry.
            viewport.window_cache.defer_pending_request();
        }
        let mut sync_requested = false;
        let mut input_codec = HostInputCodec::new();
        let mut copy_key_lease = CopyKeyLease::default();
        let mut deferred_active = false;

        let loop_result = 'terminal: loop {
            let now = Instant::now();
            if viewport_pacer.due(now) {
                if let Err(error) = present_cached_viewport_stdout(
                    &surface,
                    &mut presenter,
                    &mut viewport,
                    &status_renderer,
                    transport_state,
                    &mut viewport_pacer,
                    CachedPresentationRequest { now, force: false },
                ) {
                    break Err(error);
                }
                continue;
            }
            if prefix
                .deadline()
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                if let Some(bytes) = prefix.flush_pending() {
                    if viewport.is_live() && transport_state == TerminalViewTransportState::Active {
                        if let Err(error) = writer.write_input(bytes).await {
                            break Err(error.into());
                        }
                    } else if !viewport.is_live() {
                        let effect = viewport.retain_or_resume(bytes)?;
                        if apply_viewport_effect(
                            effect,
                            &mut viewport,
                            &surface,
                            &mut presenter,
                            &writer,
                            &status_renderer,
                            transport_state,
                            &mut viewport_pacer,
                            false,
                        )
                        .await?
                        {
                            sync_requested = true;
                        }
                    }
                }
                continue;
            }
            let prefix_deadline = prefix.deadline();
            let viewport_deadline = viewport_pacer.deadline();
            tokio::select! {
                cancellation = receive_terminal_cancellation(cancellation_receiver) => {
                    viewport_pacer.cancel();
                    break Err(cancellation.error(Some(session_id)));
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
                    physical_size = latest_physical;
                    let layout = ChromeLayout::new(
                        latest_physical,
                        remote_request,
                        viewport.effective_screen(surface.active_screen()),
                    );
                    let latest = layout.child;
                    viewport_pacer.cancel();
                    viewport.set_layout(layout);
                    status_renderer.resize(latest_physical);
                    selection.cancel();
                    reconcile_presenter_selection(
                        &mut selection,
                        &viewport,
                        &surface,
                        &mut presenter,
                    );
                    if let Err(error) = present_surface_stdout(
                        &surface,
                        &mut presenter,
                        &viewport,
                        &status_renderer,
                        transport_state,
                    ) {
                        break Err(error);
                    }
                    viewport.observe_presentation();
                    viewport_pacer.mark_presented(Instant::now());
                    if let Some(size) = resize_coalescer.observe(latest, transport_state) {
                        if let Err(error) = writer.resize(size).await {
                            break Err(error.into());
                        }
                        match apply_transport_state_transition(
                            stdin,
                            &input_epoch,
                            &mut current_input_epoch,
                            &mut stdin_pump,
                            &mut prefix,
                            transport_state,
                            TerminalViewTransportState::Synchronizing,
                            &mut viewport,
                            &surface,
                            &mut presenter,
                            &mut selection,
                            &mut status_renderer,
                            &mut resize_coalescer,
                            &writer,
                            &mut viewport_pacer,
                        ).await {
                            Ok(state) => transport_state = state,
                            Err(error) => break 'terminal Err(error),
                        }
                    }
                }
                // The explicit expired-deadline check above makes this local
                // timeout independent of continuously ready terminal output.
                () = wait_for_prefix_deadline(prefix_deadline), if prefix_deadline.is_some() => {
                    if let Some(bytes) = prefix.flush_pending() {
                        if viewport.is_live()
                            && transport_state == TerminalViewTransportState::Active
                        {
                            if let Err(error) = writer.write_input(bytes).await {
                                break Err(error.into());
                            }
                        } else if !viewport.is_live() {
                            let effect = viewport.retain_or_resume(bytes)?;
                            if apply_viewport_effect(
                                effect,
                                &mut viewport,
                                &surface,
                                &mut presenter,
                                &writer,
                                &status_renderer,
                                transport_state,
                                &mut viewport_pacer,
                                false,
                            ).await? {
                                sync_requested = true;
                            }
                        }
                    }
                }
                () = wait_for_viewport_deadline(viewport_deadline), if viewport_deadline.is_some() => {
                    let now = Instant::now();
                    if let Err(error) = present_cached_viewport_stdout(
                        &surface,
                        &mut presenter,
                        &mut viewport,
                        &status_renderer,
                        transport_state,
                        &mut viewport_pacer,
                        CachedPresentationRequest { now, force: false },
                    ) {
                        break Err(error);
                    }
                }
                event = events.read_event() => {
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
                    match event {
                        TerminalViewEvent::TransportState(state) => {
                            if should_defer_active_for_paste(state, &viewport, &input_codec) {
                                deferred_active = true;
                                continue;
                            }
                            deferred_active = false;
                            match apply_transport_state_transition(
                                stdin,
                                &input_epoch,
                                &mut current_input_epoch,
                                &mut stdin_pump,
                                &mut prefix,
                                transport_state,
                                state,
                                &mut viewport,
                                &surface,
                                &mut presenter,
                                &mut selection,
                                &mut status_renderer,
                                &mut resize_coalescer,
                                &writer,
                                &mut viewport_pacer,
                            ).await {
                                Ok(applied) => transport_state = applied,
                                Err(error) => break 'terminal Err(error),
                            }
                        }
                        TerminalViewEvent::ConnectionStatus(status) => {
                            status_renderer.observe(status)?;
                            if let Err(error) = present_surface_stdout(
                                &surface,
                                &mut presenter,
                                &viewport,
                                &status_renderer,
                                transport_state,
                            ) {
                                break Err(error);
                            }
                            viewport.observe_presentation();
                            viewport_pacer.mark_presented(Instant::now());
                        }
                        TerminalViewEvent::Snapshot(snapshot) => {
                            viewport_pacer.cancel();
                            selection.cancel();
                            reconcile_presenter_selection(
                                &mut selection,
                                &viewport,
                                &surface,
                                &mut presenter,
                            );
                            let preserving_resume_input = viewport.is_resume_pending();
                            if transport_state != TerminalViewTransportState::Synchronizing
                                && !preserving_resume_input
                            {
                                if let Err(error) = transition_input_state(
                                    stdin,
                                    &input_epoch,
                                    &mut current_input_epoch,
                                    &mut stdin_pump,
                                    &mut prefix,
                                    TerminalViewTransportState::Synchronizing,
                                ) {
                                    break 'terminal Err(error);
                                }
                                transport_state = TerminalViewTransportState::Synchronizing;
                            }
                            viewport.observe_snapshot(snapshot.surface.scroll_metrics);
                            let layout = ChromeLayout::new(
                                physical_size,
                                remote_request,
                                viewport.effective_screen(snapshot.surface.active_screen),
                            );
                            viewport.set_layout(layout);
                            let _ = resize_coalescer.observe(layout.child, transport_state);
                            let history_refill = viewport.refetch_history_window();
                            let rendered = install_snapshot_stdout(
                                &mut surface,
                                &mut presenter,
                                &snapshot,
                                &viewport,
                                &status_renderer,
                                transport_state,
                            );
                            if let Err(error) = rendered {
                                break Err(error);
                            }
                            viewport.observe_presentation();
                            viewport_pacer.mark_presented(Instant::now());
                            prefix.clear_pending();
                            sync_requested = false;
                            if let Err(error) = writer.snapshot_applied(snapshot.revision).await {
                                break Err(error.into());
                            }
                            if let Some(query) = history_refill
                                && let Err(error) = writer.request_history_window(query).await
                            {
                                break Err(error.into());
                            }
                        }
                        TerminalViewEvent::Delta(delta) => {
                            let rendered_live = viewport.is_live();
                            if rendered_live {
                                viewport_pacer.cancel();
                            }
                            let delta_result = apply_delta_stdout(
                                &mut surface,
                                &mut presenter,
                                &delta,
                                &mut viewport,
                                &mut selection,
                                &status_renderer,
                                transport_state,
                            );
                            match delta_result {
                                Ok(DeltaRender::Applied) => {
                                    sync_requested = false;
                                    if rendered_live {
                                        viewport.observe_presentation();
                                        viewport_pacer.mark_presented(Instant::now());
                                        let mode_resize = resize_coalescer
                                            .observe(viewport.content_size(), transport_state);
                                        if let Some(size) = mode_resize {
                                            if let Err(error) = writer.resize(size).await {
                                                break Err(error.into());
                                            }
                                            match apply_transport_state_transition(
                                                stdin,
                                                &input_epoch,
                                                &mut current_input_epoch,
                                                &mut stdin_pump,
                                                &mut prefix,
                                                transport_state,
                                                TerminalViewTransportState::Synchronizing,
                                                &mut viewport,
                                                &surface,
                                                &mut presenter,
                                                &mut selection,
                                                &mut status_renderer,
                                                &mut resize_coalescer,
                                                &writer,
                                                &mut viewport_pacer,
                                            ).await {
                                                Ok(state) => transport_state = state,
                                                Err(error) => break 'terminal Err(error),
                                            }
                                        }
                                    } else {
                                        cancel_unpresentable_cached_viewport(
                                            &viewport,
                                            &mut viewport_pacer,
                                        );
                                    }
                                    if transport_state == TerminalViewTransportState::Synchronizing
                                        && let Err(error) = writer
                                            .snapshot_applied(delta.to_revision)
                                            .await
                                    {
                                        break Err(error.into());
                                    }
                                }
                                Ok(DeltaRender::Gap) => {
                                    viewport_pacer.cancel();
                                    selection.cancel();
                                    reconcile_presenter_selection(
                                        &mut selection,
                                        &viewport,
                                        &surface,
                                        &mut presenter,
                                    );
                                    if rendered_live {
                                        viewport.begin_resume(Vec::new())?;
                                    }
                                    if transport_state
                                        != TerminalViewTransportState::Synchronizing
                                        && !viewport.is_resume_pending()
                                    {
                                        if let Err(error) = transition_input_state(
                                            stdin,
                                            &input_epoch,
                                            &mut current_input_epoch,
                                            &mut stdin_pump,
                                            &mut prefix,
                                            TerminalViewTransportState::Synchronizing,
                                        ) {
                                            break 'terminal Err(error);
                                        }
                                        transport_state =
                                            TerminalViewTransportState::Synchronizing;
                                    }
                                    if !sync_requested {
                                        sync_requested = true;
                                        if let Err(error) = writer
                                            .request_sync(surface.revision())
                                            .await
                                        {
                                            break 'terminal Err(error.into());
                                        }
                                    }
                                }
                                Err(error) => break Err(error),
                            }
                        }
                        TerminalViewEvent::HistoryWindow(result) => {
                            let effect = viewport.apply_view_history_window(result)?;
                            reconcile_presenter_selection_for_next_frame(
                                &mut selection,
                                &viewport,
                                &surface,
                                &mut presenter,
                            );
                            if apply_viewport_effect(
                                effect,
                                &mut viewport,
                                &surface,
                                &mut presenter,
                                &writer,
                                &status_renderer,
                                transport_state,
                                &mut viewport_pacer,
                                false,
                            )
                            .await?
                            {
                                sync_requested = true;
                            }
                            reconcile_presenter_selection(
                                &mut selection,
                                &viewport,
                                &surface,
                                &mut presenter,
                            );
                        }
                        TerminalViewEvent::ClipboardWrite(write) => {
                            let stdout = io::stdout();
                            let mut output = stdout.lock();
                            if let Err(error) = presenter.write_clipboard(&mut output, &write) {
                                break Err(error);
                            }
                        }
                        TerminalViewEvent::SyncRequired { .. } => {
                            viewport_pacer.cancel();
                            selection.cancel();
                            reconcile_presenter_selection(
                                &mut selection,
                                &viewport,
                                &surface,
                                &mut presenter,
                            );
                            viewport.observe_sync_required();
                            if transport_state != TerminalViewTransportState::Synchronizing {
                                transport_state = TerminalViewTransportState::Synchronizing;
                            }
                            // The marker and its authoritative replacement snapshot are emitted
                            // together. Keep the last complete host presentation untouched while
                            // that replacement is in flight instead of repainting an identical
                            // history frame or clearing attachment scroll state with a redundant
                            // sync request.
                            sync_requested = true;
                        }
                        TerminalViewEvent::LeaseLost { .. } => {
                            viewport_pacer.cancel();
                            break Err(terminal_daemon_error(
                                DomainErrorKind::LeaseLost,
                                "another attachment took over this terminal controller",
                            ));
                        }
                        TerminalViewEvent::SessionEnded(ended) => {
                            viewport_pacer.cancel();
                            break terminal_end_completion(ended.reason);
                        }
                    }
                }
                input = stdin_pump.recv() => {
                    match input {
                        Some(StdinEvent::Bytes { epoch, bytes })
                            if input_epoch_is_current(epoch, current_input_epoch) =>
                        {
                            // A paced history frame may have committed a new
                            // source since the preceding pointer event. Retire
                            // any old coordinates before interpreting a copy
                            // key against the physical keyboard mode.
                            reconcile_presenter_selection(
                                &mut selection,
                                &viewport,
                                &surface,
                                &mut presenter,
                            );
                            let mut host_events = match input_codec.feed(&bytes) {
                                Ok(events) => VecDeque::from(events),
                                Err(error) => break 'terminal Err(error),
                            };
                            let mut force_viewport_presentation = false;
                            while let Some(host_event) = host_events.pop_front() {
                                match host_event {
                                    HostInputEvent::Bytes(bytes) => {
                                        if let Err(error) = invalidate_selection_stdout(
                                            &mut selection,
                                            &mut viewport,
                                            &surface,
                                            &mut presenter,
                                            &status_renderer,
                                            transport_state,
                                            &mut viewport_pacer,
                                        ) {
                                            break 'terminal Err(error);
                                        }
                                        for action in prefix.feed(&bytes, Instant::now()) {
                                            match action {
                                                PrefixAction::Input(bytes) if viewport.is_live()
                                                    && transport_state
                                                        == TerminalViewTransportState::Active =>
                                                {
                                                    if let Err(error) = writer.write_input(bytes).await {
                                                        break 'terminal Err(error.into());
                                                    }
                                                }
                                                PrefixAction::Input(bytes) if !viewport.is_live() => {
                                                    let effect = viewport.retain_or_resume(bytes)?;
                                                    if apply_viewport_effect(
                                                        effect,
                                                        &mut viewport,
                                                        &surface,
                                                        &mut presenter,
                                                        &writer,
                                                        &status_renderer,
                                                        transport_state,
                                                        &mut viewport_pacer,
                                                        true,
                                                    ).await? {
                                                        sync_requested = true;
                                                    }
                                                }
                                                PrefixAction::Input(_) => {}
                                                PrefixAction::Detach => break,
                                            }
                                        }
                                    }
                                    HostInputEvent::LegacyCtrlC => {
                                        if selection.is_finalized() {
                                            if let Err(error) = write_selection_clipboard_stdout(
                                                &selection,
                                                &viewport,
                                                &surface,
                                                &mut presenter,
                                            ) {
                                                break 'terminal Err(error);
                                            }
                                        } else {
                                            host_events
                                                .push_front(HostInputEvent::Bytes(vec![0x03]));
                                        }
                                    }
                                    HostInputEvent::EnhancedKey(key) => {
                                        let outer_flags = presenter.presented_keyboard_flags();
                                        match route_enhanced_input(
                                            &key,
                                            surface.modes(),
                                            outer_flags,
                                            selection.is_finalized(),
                                            &mut copy_key_lease,
                                        ) {
                                            KeyboardRoute::Copy => {
                                                if let Err(error) =
                                                    write_selection_clipboard_stdout(
                                                        &selection,
                                                        &viewport,
                                                        &surface,
                                                        &mut presenter,
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
                                                            &mut selection,
                                                            &mut viewport,
                                                            &surface,
                                                            &mut presenter,
                                                            &status_renderer,
                                                            transport_state,
                                                            &mut viewport_pacer,
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
                                            &mut selection,
                                            &mut viewport,
                                            &surface,
                                            &mut presenter,
                                            &status_renderer,
                                            transport_state,
                                            &mut viewport_pacer,
                                        ) {
                                            break 'terminal Err(error);
                                        }
                                        if viewport.is_live()
                                            && transport_state
                                                == TerminalViewTransportState::Active
                                        {
                                            if let Err(error) = writer.write_input(bytes).await {
                                                break 'terminal Err(error.into());
                                            }
                                        } else if !viewport.is_live() {
                                            let effect = viewport.retain_or_resume(bytes)?;
                                            if apply_viewport_effect(
                                                effect,
                                                &mut viewport,
                                                &surface,
                                                &mut presenter,
                                                &writer,
                                                &status_renderer,
                                                transport_state,
                                                &mut viewport_pacer,
                                                true,
                                            ).await? {
                                                sync_requested = true;
                                            }
                                        }
                                    }
                                    HostInputEvent::PageUp | HostInputEvent::PageDown => {
                                        if let Err(error) = invalidate_selection_stdout(
                                            &mut selection,
                                            &mut viewport,
                                            &surface,
                                            &mut presenter,
                                            &status_renderer,
                                            transport_state,
                                            &mut viewport_pacer,
                                        ) {
                                            break 'terminal Err(error);
                                        }
                                        let older = matches!(host_event, HostInputEvent::PageUp);
                                        let raw = if older { PAGE_UP } else { PAGE_DOWN };
                                        if viewport.is_resume_pending() {
                                            viewport.retain_resume_input(raw)?;
                                        } else if viewport.is_history()
                                            || live_history_navigation_allowed(transport_state)
                                                && history_owns_gestures(
                                                    surface.active_screen(),
                                                    surface.modes(),
                                                )
                                        {
                                            let effect = viewport.navigate(
                                                older,
                                                usize::from(viewport.content_size().rows)
                                                    .saturating_sub(1)
                                                    .max(1),
                                            );
                                            if apply_viewport_effect(
                                                effect,
                                                &mut viewport,
                                                &surface,
                                                &mut presenter,
                                                &writer,
                                                &status_renderer,
                                                transport_state,
                                                &mut viewport_pacer,
                                                true,
                                            ).await? {
                                                sync_requested = true;
                                            }
                                        } else if transport_state
                                            == TerminalViewTransportState::Active
                                            && let Err(error) = writer.write_input(raw.to_vec()).await
                                        {
                                            break 'terminal Err(error.into());
                                        }
                                    }
                                    HostInputEvent::Mouse(mouse) => {
                                        let routed = match route_pointer(
                                            &mouse,
                                            &mut viewport,
                                            &surface,
                                            &mut selection,
                                            live_history_navigation_allowed(transport_state),
                                        ) {
                                            Ok(routed) => routed,
                                            Err(error) => break 'terminal Err(error),
                                        };
                                        reconcile_presenter_selection(
                                            &mut selection,
                                            &viewport,
                                            &surface,
                                            &mut presenter,
                                        );
                                        match routed {
                                            PointerRoute::Viewport(effect) => {
                                                force_viewport_presentation |= mouse.release;
                                                if apply_viewport_effect(
                                                    effect,
                                                    &mut viewport,
                                                    &surface,
                                                    &mut presenter,
                                                    &writer,
                                                    &status_renderer,
                                                    transport_state,
                                                    &mut viewport_pacer,
                                                    true,
                                                ).await? {
                                                    sync_requested = true;
                                                }
                                            }
                                            PointerRoute::Child(bytes)
                                                if viewport.is_resume_pending() =>
                                            {
                                                viewport.retain_resume_input(&bytes)?;
                                            }
                                            PointerRoute::Child(bytes)
                                                if viewport.is_live()
                                                    && transport_state
                                                        == TerminalViewTransportState::Active =>
                                            {
                                                if let Err(error) = writer.write_input(bytes).await {
                                                    break 'terminal Err(error.into());
                                                }
                                            }
                                            PointerRoute::Child(_) | PointerRoute::Ignore => {}
                                            PointerRoute::SelectionChanged => {
                                                let now = Instant::now();
                                                if mark_cached_viewport_dirty(
                                                    &viewport,
                                                    &mut viewport_pacer,
                                                    now,
                                                ) {
                                                    force_viewport_presentation |= mouse.release;
                                                }
                                            }
                                        }
                                    }
                                }
                                if prefix.detached() {
                                    break;
                                }
                            }
                            if prefix.detached() {
                                viewport_pacer.cancel();
                                break Ok(TerminalCompletion::Detached);
                            }
                            if deferred_active && !input_codec.paste_in_progress() {
                                match apply_transport_state_transition(
                                    stdin,
                                    &input_epoch,
                                    &mut current_input_epoch,
                                    &mut stdin_pump,
                                    &mut prefix,
                                    transport_state,
                                    TerminalViewTransportState::Active,
                                    &mut viewport,
                                    &surface,
                                    &mut presenter,
                                    &mut selection,
                                    &mut status_renderer,
                                    &mut resize_coalescer,
                                    &writer,
                                    &mut viewport_pacer,
                                ).await {
                                    Ok(applied) => transport_state = applied,
                                    Err(error) => break 'terminal Err(error),
                                }
                                deferred_active = false;
                            }
                            if let Err(error) = present_cached_viewport_stdout(
                                &surface,
                                &mut presenter,
                                &mut viewport,
                                &status_renderer,
                                transport_state,
                                &mut viewport_pacer,
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
                            viewport_pacer.cancel();
                            if let Some(bytes) = take_pending_active_input(
                                &mut prefix,
                                transport_state,
                            ) && viewport.is_live()
                                && let Err(error) = writer.write_input(bytes).await
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
        };

        finish_terminal_view(loop_result, &mut stdin_pump, &writer).await
    }

    struct InactiveWaitContext<'a> {
        stdout: &'a io::Stdout,
        resize_signal: &'a mut Signal,
        cancellation_receiver: &'a mut watch::Receiver<Option<TerminalSignalCancellation>>,
        stdin_pump: &'a mut StdinPump,
        prefix: &'a mut PrefixParser,
        resize_coalescer: &'a mut ResizeCoalescer,
        current_input_epoch: u64,
        preserve_submitted_result: bool,
        remote: bool,
    }

    async fn await_while_inactive<T>(
        future: impl Future<Output = Result<T, CliError>>,
        context: InactiveWaitContext<'_>,
    ) -> Result<InactiveWait<T>, CliError> {
        let InactiveWaitContext {
            stdout,
            resize_signal,
            cancellation_receiver,
            stdin_pump,
            prefix,
            resize_coalescer,
            current_input_epoch,
            preserve_submitted_result,
            remote,
        } = context;
        tokio::pin!(future);
        loop {
            if prefix
                .deadline()
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                let _ = prefix.flush_pending();
                continue;
            }
            let prefix_deadline = prefix.deadline();
            tokio::select! {
                biased;
                result = &mut future => return result.map(InactiveWait::Ready),
                cancellation = receive_terminal_cancellation(cancellation_receiver) => {
                    let cancellation = InactiveCancellation::Signal(cancellation);
                    if preserve_submitted_result {
                        return finish_submitted_after_cancellation(&mut future, cancellation).await;
                    }
                    return Ok(InactiveWait::Cancelled(cancellation));
                }
                signal = resize_signal.recv() => {
                    if signal.is_none() {
                        return Err(terminal_daemon_error(
                            DomainErrorKind::Cancelled,
                            "SIGWINCH handler closed",
                        ));
                    }
                    let latest = child_terminal_size(terminal_size(stdout)?, remote);
                    let _ = resize_coalescer.observe(
                        latest,
                        TerminalViewTransportState::Synchronizing,
                    );
                }
                () = wait_for_prefix_deadline(prefix_deadline), if prefix_deadline.is_some() => {
                    let _ = prefix.flush_pending();
                }
                input = stdin_pump.recv() => {
                    match input {
                        Some(StdinEvent::Bytes { epoch, bytes })
                            if input_epoch_is_current(epoch, current_input_epoch) =>
                        {
                            for action in prefix.feed(&bytes, Instant::now()) {
                                if action == PrefixAction::Detach {
                                    let cancellation = InactiveCancellation::LocalDetach;
                                    if preserve_submitted_result {
                                        return finish_submitted_after_cancellation(
                                            &mut future,
                                            cancellation,
                                        )
                                        .await;
                                    }
                                    return Ok(InactiveWait::Cancelled(cancellation));
                                }
                            }
                        }
                        Some(StdinEvent::Bytes { .. }) => {}
                        Some(StdinEvent::Eof) | None => {
                            prefix.clear_pending();
                            let cancellation = InactiveCancellation::LocalDetach;
                            if preserve_submitted_result {
                                return finish_submitted_after_cancellation(
                                    &mut future,
                                    cancellation,
                                )
                                .await;
                            }
                            return Ok(InactiveWait::Cancelled(cancellation));
                        }
                        Some(StdinEvent::Error(detail)) => {
                            return Err(CliError::Io(format!(
                                "read terminal stdin: {detail}"
                            )));
                        }
                    }
                }
            }
        }
    }

    async fn finish_submitted_after_cancellation<T>(
        future: impl Future<Output = Result<T, CliError>>,
        cancellation: InactiveCancellation,
    ) -> Result<InactiveWait<T>, CliError> {
        future
            .await
            .map(|value| InactiveWait::CompletedAfterCancellation {
                value,
                cancellation,
            })
    }

    fn transition_input_state(
        stdin: &impl AsFd,
        input_epoch: &InputEpoch,
        current_input_epoch: &mut u64,
        stdin_pump: &mut StdinPump,
        prefix: &mut PrefixParser,
        next: TerminalViewTransportState,
    ) -> Result<(), CliError> {
        if next == TerminalViewTransportState::Active {
            return stdin_pump.replace_after_active_fence(
                stdin,
                input_epoch,
                current_input_epoch,
                prefix,
            );
        }
        *current_input_epoch = input_epoch.advance();
        prefix.clear_pending();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn transition_transport_input_state(
        stdin: &impl AsFd,
        input_epoch: &InputEpoch,
        current_input_epoch: &mut u64,
        stdin_pump: &mut StdinPump,
        prefix: &mut PrefixParser,
        previous: TerminalViewTransportState,
        next: TerminalViewTransportState,
        viewport: &mut ViewportController,
    ) -> Result<Option<Vec<u8>>, CliError> {
        if next != previous
            && (!viewport.is_resume_pending() || next == TerminalViewTransportState::Active)
        {
            transition_input_state(
                stdin,
                input_epoch,
                current_input_epoch,
                stdin_pump,
                prefix,
                next,
            )?;
        }
        Ok((next == TerminalViewTransportState::Active)
            .then(|| viewport.finish_resume())
            .flatten())
    }

    fn should_defer_active_for_paste(
        next: TerminalViewTransportState,
        viewport: &ViewportController,
        input_codec: &HostInputCodec,
    ) -> bool {
        next == TerminalViewTransportState::Active
            && viewport.is_resume_pending()
            && input_codec.paste_in_progress()
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_transport_state_transition(
        stdin: &impl AsFd,
        input_epoch: &InputEpoch,
        current_input_epoch: &mut u64,
        stdin_pump: &mut StdinPump,
        prefix: &mut PrefixParser,
        previous: TerminalViewTransportState,
        next: TerminalViewTransportState,
        viewport: &mut ViewportController,
        surface: &AttachmentSurface,
        presenter: &mut DesktopPresenter,
        selection: &mut SelectionController,
        status_renderer: &mut StatusRenderer,
        resize_coalescer: &mut ResizeCoalescer,
        writer: &zterm_daemon::operations::TerminalViewCommandWriter,
        viewport_pacer: &mut ViewportPresentationPacer,
    ) -> Result<TerminalViewTransportState, CliError> {
        viewport_pacer.cancel();
        let (next, pending_resize) = resize_coalescer.enter_transport_state(next);
        if next == TerminalViewTransportState::Reconnecting {
            viewport.reset_presentation_for_reconnect();
            status_renderer.reset_for_reconnect();
        }
        if next != previous && next != TerminalViewTransportState::Active {
            selection.cancel();
        }
        let resume_input = transition_transport_input_state(
            stdin,
            input_epoch,
            current_input_epoch,
            stdin_pump,
            prefix,
            previous,
            next,
            viewport,
        )?;
        reconcile_presenter_selection(selection, viewport, surface, presenter);
        if present_transport_transition_stdout(
            surface,
            presenter,
            viewport,
            status_renderer,
            next,
            resume_input.is_some(),
        )? {
            viewport.observe_presentation();
            viewport_pacer.mark_presented(Instant::now());
        }
        if let Some(size) = pending_resize {
            writer.resize(size).await?;
        }
        if let Some(bytes) = resume_input
            && !bytes.is_empty()
        {
            writer.write_input(bytes).await?;
        }
        Ok(next)
    }

    const fn input_epoch_is_current(observed: u64, current: u64) -> bool {
        observed == current
    }

    async fn finish_terminal_view(
        loop_result: Result<TerminalCompletion, CliError>,
        stdin_pump: &mut StdinPump,
        writer: &zterm_daemon::operations::TerminalViewCommandWriter,
    ) -> Result<TerminalCompletion, CliError> {
        let stdin_result = stdin_pump.shutdown();
        let session_already_ended = matches!(&loop_result, Ok(TerminalCompletion::SessionEnded(_)));
        let detach_result = if session_already_ended {
            Ok(())
        } else {
            match tokio::time::timeout(DETACH_TIMEOUT, writer.detach()).await {
                Ok(result) => result.map_err(CliError::from),
                Err(_) => Err(terminal_daemon_error(
                    DomainErrorKind::DeadlineExceeded,
                    "timed out detaching the local terminal view",
                )),
            }
        };
        match (loop_result, stdin_result, detach_result) {
            (Err(error), _, _) => Err(error),
            (Ok(_), Err(error), _) => Err(error),
            (Ok(_), Ok(()), Err(error)) => Err(error),
            (Ok(completion), Ok(()), Ok(())) => Ok(completion),
        }
    }

    async fn prepare(
        request: TerminalRequest,
        runtime: &LocalRuntime,
        viewport: TerminalSize,
    ) -> Result<PreparedTerminalView, CliError> {
        match request.kind {
            TerminalRequestKind::Attach {
                target,
                selector,
                create_main,
                takeover,
            } => runtime
                .attach(
                    &target,
                    selector.as_deref(),
                    create_main,
                    takeover,
                    Some(viewport),
                )
                .await
                .map_err(Into::into),
            TerminalRequestKind::Create {
                target,
                name,
                working_directory,
            } => {
                let created = runtime
                    .session_create_for_attach(
                        &target,
                        &name,
                        working_directory.as_deref(),
                        Some(viewport),
                    )
                    .await?;
                let session_id = created.summary().session_id;
                preserve_created_session(
                    session_id,
                    runtime.attach_created(&created, Some(viewport)).await,
                )
            }
        }
    }

    fn terminal_request_is_remote(request: &TerminalRequest) -> bool {
        match &request.kind {
            TerminalRequestKind::Attach { target, .. }
            | TerminalRequestKind::Create { target, .. } => target != RESERVED_DEVICE_ALIAS,
        }
    }

    fn child_terminal_size(physical: TerminalSize, remote: bool) -> TerminalSize {
        ChromeLayout::new(physical, remote, ActiveScreen::Main).child
    }

    fn preserve_created_session<T>(
        session_id: zterm_core::SessionId,
        result: Result<T, DaemonError>,
    ) -> Result<T, CliError> {
        result.map_err(|source| CliError::CreatedSessionAttach { session_id, source })
    }

    struct TerminalSignals {
        resize: Signal,
        interrupt: Signal,
        terminate: Signal,
        hangup: Signal,
    }

    impl TerminalSignals {
        fn install() -> Result<Self, CliError> {
            Ok(Self {
                resize: signal(SignalKind::window_change())
                    .map_err(|error| terminal_io("install SIGWINCH handler", error))?,
                interrupt: signal(SignalKind::interrupt())
                    .map_err(|error| terminal_io("install SIGINT handler", error))?,
                terminate: signal(SignalKind::terminate())
                    .map_err(|error| terminal_io("install SIGTERM handler", error))?,
                hangup: signal(SignalKind::hangup())
                    .map_err(|error| terminal_io("install SIGHUP handler", error))?,
            })
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TerminalSignalCancellation {
        name: &'static str,
        received: bool,
    }

    impl TerminalSignalCancellation {
        const fn new(name: &'static str, received: Option<()>) -> Self {
            Self {
                name,
                received: received.is_some(),
            }
        }

        fn error(self, prepared_session: Option<SessionId>) -> CliError {
            let detail = match (self.received, prepared_session) {
                (true, Some(session_id)) => format!(
                    "terminal attachment interrupted by {} after Session {session_id} was prepared; the Session remains live",
                    self.name
                ),
                (false, Some(session_id)) => format!(
                    "{} handler closed after Session {session_id} was prepared; the Session remains live",
                    self.name
                ),
                (true, None) => format!("terminal attachment interrupted by {}", self.name),
                (false, None) => format!("{} handler closed", self.name),
            };
            terminal_daemon_error(DomainErrorKind::Cancelled, &detail)
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum InactiveCancellation {
        LocalDetach,
        Signal(TerminalSignalCancellation),
    }

    enum InactiveWait<T> {
        Ready(T),
        Cancelled(InactiveCancellation),
        CompletedAfterCancellation {
            value: T,
            cancellation: InactiveCancellation,
        },
    }

    fn current_terminal_cancellation(
        receiver: &mut watch::Receiver<Option<TerminalSignalCancellation>>,
    ) -> Option<TerminalSignalCancellation> {
        *receiver.borrow_and_update()
    }

    async fn receive_terminal_cancellation(
        receiver: &mut watch::Receiver<Option<TerminalSignalCancellation>>,
    ) -> TerminalSignalCancellation {
        loop {
            if let Some(cancellation) = current_terminal_cancellation(receiver) {
                return cancellation;
            }
            if receiver.changed().await.is_err() {
                return TerminalSignalCancellation::new("terminal signal coordinator", None);
            }
        }
    }

    fn inactive_cancellation_result(
        cancellation: InactiveCancellation,
        prepared_session: Option<SessionId>,
    ) -> Result<TerminalCompletion, CliError> {
        match cancellation {
            InactiveCancellation::LocalDetach => Ok(prepared_session.map_or(
                TerminalCompletion::Detached,
                TerminalCompletion::PreparedThenDetached,
            )),
            InactiveCancellation::Signal(signal) => Err(signal.error(prepared_session)),
        }
    }

    fn terminal_daemon_error(kind: DomainErrorKind, detail: &str) -> CliError {
        CliError::Daemon(DaemonError::new(kind, detail))
    }

    fn semantic_surface_error(detail: &str) -> CliError {
        terminal_daemon_error(DomainErrorKind::MalformedFrame, detail)
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TerminalCompletion {
        Detached,
        PreparedThenDetached(SessionId),
        SessionEnded(TerminalViewEndReason),
    }

    fn terminal_end_completion(
        reason: TerminalViewEndReason,
    ) -> Result<TerminalCompletion, CliError> {
        if reason == TerminalViewEndReason::DriverFailure {
            Err(CliError::TerminalDriverFailure)
        } else {
            Ok(TerminalCompletion::SessionEnded(reason))
        }
    }

    fn emit_completion_diagnostic(completion: TerminalCompletion) -> Result<(), CliError> {
        let Some(message) = completion_diagnostic(completion)? else {
            return Ok(());
        };
        write_all_fd(&io::stderr(), &message)
            .map_err(|error| terminal_io("write terminal completion diagnostic", error))
    }

    fn completion_diagnostic(completion: TerminalCompletion) -> Result<Option<Vec<u8>>, CliError> {
        Ok(Some(match completion {
            TerminalCompletion::Detached => return Ok(None),
            TerminalCompletion::PreparedThenDetached(session_id) => format!(
                "zterm: detached after Session {session_id} was prepared; it remains live.\n"
            )
            .into_bytes(),
            TerminalCompletion::SessionEnded(TerminalViewEndReason::NaturalExit) => {
                b"zterm: Session ended after its root shell exited.\n".to_vec()
            }
            TerminalCompletion::SessionEnded(TerminalViewEndReason::ExplicitClose) => {
                b"zterm: Session was closed explicitly.\n".to_vec()
            }
            TerminalCompletion::SessionEnded(TerminalViewEndReason::DaemonStop) => {
                b"zterm: Session ended because its daemon stopped.\n".to_vec()
            }
            TerminalCompletion::SessionEnded(TerminalViewEndReason::DriverFailure) => {
                return Err(CliError::TerminalDriverFailure);
            }
        }))
    }

    fn terminal_io(operation: &'static str, error: io::Error) -> CliError {
        CliError::Io(format!("{operation}: {error}"))
    }

    fn terminal_size(terminal: &impl AsFd) -> Result<TerminalSize, CliError> {
        let size = rustix::termios::tcgetwinsize(terminal)
            .map_err(|error| terminal_io("read terminal viewport", error.into()))?;
        if size.ws_row == 0 || size.ws_col == 0 {
            return Err(CliError::Usage(
                "interactive terminal viewport must have non-zero rows and columns".to_owned(),
            ));
        }
        Ok(TerminalSize::new(size.ws_row, size.ws_col))
    }

    type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

    struct ScopedPanicHook {
        previous: Option<PanicHook>,
    }

    impl ScopedPanicHook {
        fn suppress() -> Self {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            Self {
                previous: Some(previous),
            }
        }
    }

    impl Drop for ScopedPanicHook {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                std::panic::set_hook(previous);
            }
        }
    }

    struct TerminalGuard {
        input: OwnedFd,
        output: OwnedFd,
        original: Termios,
        restored: bool,
    }

    impl TerminalGuard {
        fn enter(input: &impl AsFd, output: &impl AsFd) -> Result<Self, CliError> {
            let input = duplicate_cloexec(input, "duplicate terminal stdin")?;
            let output = duplicate_cloexec(output, "duplicate terminal stdout")?;
            let original = tcgetattr_retry(&input)
                .map_err(|error| terminal_io("read terminal attributes", error.into()))?;
            let mut raw = original.clone();
            cfmakeraw(&mut raw);
            tcsetattr_retry(&input, SetArg::TCSANOW, &raw)
                .map_err(|error| terminal_io("enable terminal raw mode", error.into()))?;
            let mut guard = Self {
                input,
                output,
                original,
                restored: false,
            };
            if let Err(error) = write_all_fd(&guard.output, ENTER_TERMINAL_UI) {
                let _ = guard.restore();
                return Err(terminal_io("enter terminal UI", error));
            }
            Ok(guard)
        }

        fn restore(&mut self) -> Result<(), CliError> {
            if self.restored {
                return Ok(());
            }
            let output = write_all_fd(&self.output, RESTORE_TERMINAL_UI)
                .map_err(|error| terminal_io("restore terminal display", error));
            let attributes = tcsetattr_retry(&self.input, SetArg::TCSANOW, &self.original)
                .map_err(|error| terminal_io("restore terminal attributes", error.into()));
            if output.is_ok() && attributes.is_ok() {
                self.restored = true;
            }
            match (output, attributes) {
                (_, Err(error)) => Err(error),
                (Err(error), Ok(())) => Err(error),
                (Ok(()), Ok(())) => Ok(()),
            }
        }
    }

    impl Drop for TerminalGuard {
        fn drop(&mut self) {
            let _ = self.restore();
        }
    }

    fn tcgetattr_retry(fd: &impl AsFd) -> Result<Termios, nix::errno::Errno> {
        loop {
            match tcgetattr(fd) {
                Err(nix::errno::Errno::EINTR) => {}
                result => return result,
            }
        }
    }

    fn tcsetattr_retry(
        fd: &impl AsFd,
        action: SetArg,
        attributes: &Termios,
    ) -> Result<(), nix::errno::Errno> {
        loop {
            match tcsetattr(fd, action, attributes) {
                Err(nix::errno::Errno::EINTR) => {}
                result => return result,
            }
        }
    }

    fn write_all_fd(fd: &impl AsFd, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            match rustix::io::write(fd, bytes) {
                Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
                Ok(written) => bytes = &bytes[written..],
                Err(error) if error == rustix::io::Errno::INTR => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn duplicate_cloexec(fd: &impl AsFd, operation: &'static str) -> Result<OwnedFd, CliError> {
        rustix::io::fcntl_dupfd_cloexec(fd, 0).map_err(|error| terminal_io(operation, error.into()))
    }

    fn cancellation_pipe() -> Result<(OwnedFd, OwnedFd), CliError> {
        #[cfg(target_os = "linux")]
        {
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).map_err(|error| {
                terminal_io("create terminal stdin cancellation pipe", error.into())
            })
        }

        #[cfg(not(target_os = "linux"))]
        {
            // macOS has no rustix `pipe_with` boundary. Set both descriptor
            // flags through safe fcntl calls before publishing either end or
            // starting the reader thread.
            let (read, write) = rustix::pipe::pipe().map_err(|error| {
                terminal_io("create terminal stdin cancellation pipe", error.into())
            })?;
            set_cloexec(&read, "protect terminal stdin cancellation reader")?;
            set_cloexec(&write, "protect terminal stdin cancellation writer")?;
            Ok((read, write))
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn set_cloexec(fd: &impl AsFd, operation: &'static str) -> Result<(), CliError> {
        let flags =
            rustix::io::fcntl_getfd(fd).map_err(|error| terminal_io(operation, error.into()))?;
        rustix::io::fcntl_setfd(fd, flags | rustix::io::FdFlags::CLOEXEC)
            .map_err(|error| terminal_io(operation, error.into()))
    }

    #[derive(Clone)]
    struct InputEpoch(Arc<AtomicU64>);

    impl InputEpoch {
        fn new() -> Self {
            Self(Arc::new(AtomicU64::new(0)))
        }

        fn current(&self) -> u64 {
            self.0.load(Ordering::Acquire)
        }

        fn advance(&self) -> u64 {
            let previous = self
                .0
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                    value.checked_add(1)
                })
                .expect("terminal input epoch exhausted");
            previous + 1
        }
    }

    enum StdinEvent {
        Bytes { epoch: u64, bytes: Vec<u8> },
        Eof,
        Error(String),
    }

    #[cfg(test)]
    struct StdinReaderTestGate {
        observed: TestSender<()>,
        release: TestReceiver<()>,
    }

    #[cfg(test)]
    struct StdinReaderTestControl {
        observed: TestReceiver<()>,
        release: TestSender<()>,
    }

    #[cfg(test)]
    #[derive(Clone)]
    struct StdinReaderTestSeam {
        gates: Arc<Mutex<VecDeque<StdinReaderTestGate>>>,
    }

    #[cfg(test)]
    impl StdinReaderTestSeam {
        fn with_gates(count: usize) -> (Self, Vec<StdinReaderTestControl>) {
            let mut gates = VecDeque::with_capacity(count);
            let mut controls = Vec::with_capacity(count);
            for _ in 0..count {
                let (observed_sender, observed) = sync_channel(1);
                let (release, release_receiver) = sync_channel(1);
                gates.push_back(StdinReaderTestGate {
                    observed: observed_sender,
                    release: release_receiver,
                });
                controls.push(StdinReaderTestControl { observed, release });
            }
            (
                Self {
                    gates: Arc::new(Mutex::new(gates)),
                },
                controls,
            )
        }

        fn after_readable_poll(&self) -> Result<(), String> {
            let gate = self
                .gates
                .lock()
                .map_err(|_| "terminal stdin test seam was poisoned".to_owned())?
                .pop_front();
            let Some(gate) = gate else {
                return Ok(());
            };
            gate.observed
                .send(())
                .map_err(|_| "terminal stdin test observer was dropped".to_owned())?;
            gate.release
                .recv()
                .map_err(|_| "terminal stdin test release was dropped".to_owned())
        }
    }

    struct StdinPump {
        receiver: Option<mpsc::Receiver<StdinEvent>>,
        #[cfg(test)]
        sender_for_test: mpsc::Sender<StdinEvent>,
        _cancellation_read_guard: OwnedFd,
        cancellation_write: Option<OwnedFd>,
        handle: Option<JoinHandle<Result<(), String>>>,
        #[cfg(test)]
        reader_test_seam: Option<StdinReaderTestSeam>,
    }

    impl StdinPump {
        fn start(input: &impl AsFd, input_epoch: InputEpoch) -> Result<Self, CliError> {
            Self::start_inner(
                input,
                input_epoch,
                #[cfg(test)]
                None,
            )
        }

        fn start_inner(
            input: &impl AsFd,
            input_epoch: InputEpoch,
            #[cfg(test)] reader_test_seam: Option<StdinReaderTestSeam>,
        ) -> Result<Self, CliError> {
            let fd = duplicate_cloexec(input, "duplicate terminal stdin reader")?;
            let (cancellation_read_guard, cancellation_write) = cancellation_pipe()?;
            let cancellation_read = duplicate_cloexec(
                &cancellation_read_guard,
                "duplicate terminal stdin cancellation reader",
            )?;
            let (sender, receiver) = mpsc::channel(STDIN_CHANNEL_CAPACITY);
            #[cfg(test)]
            let sender_for_test = sender.clone();
            #[cfg(test)]
            let thread_reader_test_seam = reader_test_seam.clone();
            let handle = std::thread::Builder::new()
                .name("zterm-cli-stdin".to_owned())
                .spawn(move || {
                    stdin_reader(
                        fd,
                        cancellation_read,
                        sender,
                        input_epoch,
                        #[cfg(test)]
                        thread_reader_test_seam,
                    )
                })
                .map_err(|error| terminal_io("start terminal stdin reader", error))?;
            Ok(Self {
                receiver: Some(receiver),
                #[cfg(test)]
                sender_for_test,
                _cancellation_read_guard: cancellation_read_guard,
                cancellation_write: Some(cancellation_write),
                handle: Some(handle),
                #[cfg(test)]
                reader_test_seam,
            })
        }

        #[cfg(test)]
        fn start_with_test_seam(
            input: &impl AsFd,
            input_epoch: InputEpoch,
            reader_test_seam: StdinReaderTestSeam,
        ) -> Result<Self, CliError> {
            Self::start_inner(input, input_epoch, Some(reader_test_seam))
        }

        async fn recv(&mut self) -> Option<StdinEvent> {
            self.receiver
                .as_mut()
                .expect("terminal stdin receiver exists while its pump is live")
                .recv()
                .await
        }

        fn replace_after_active_fence(
            &mut self,
            input: &impl AsFd,
            input_epoch: &InputEpoch,
            current_input_epoch: &mut u64,
            prefix: &mut PrefixParser,
        ) -> Result<(), CliError> {
            #[cfg(test)]
            let reader_test_seam = self.reader_test_seam.clone();

            // This ordering is the input-safety boundary. The old receiver is
            // discarded before its reader is woken and joined; no reader exists
            // while the kernel queue is flushed, and the replacement reader is
            // not created until the new epoch and prefix state are installed.
            self.shutdown()?;
            loop {
                match tcflush(input, FlushArg::TCIFLUSH) {
                    Ok(()) => break,
                    Err(nix::errno::Errno::EINTR) => {}
                    Err(error) => {
                        return Err(terminal_io(
                            "discard unsynchronized terminal input",
                            error.into(),
                        ));
                    }
                }
            }
            *current_input_epoch = input_epoch.advance();
            prefix.clear_pending();
            let replacement = Self::start_inner(
                input,
                input_epoch.clone(),
                #[cfg(test)]
                reader_test_seam,
            )?;
            *self = replacement;
            Ok(())
        }

        fn shutdown(&mut self) -> Result<(), CliError> {
            if let Some(mut receiver) = self.receiver.take() {
                receiver.close();
                drop(receiver);
            }
            let Some(handle) = self.handle.take() else {
                return Ok(());
            };
            let wake = self
                .cancellation_write
                .take()
                .map(|writer| {
                    let result = write_all_fd(&writer, &[1])
                        .map_err(|error| terminal_io("wake terminal stdin reader", error));
                    drop(writer);
                    result
                })
                .unwrap_or(Ok(()));
            let joined = match handle.join() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(detail)) => Err(CliError::Io(detail)),
                Err(payload) => {
                    drop(payload);
                    Err(CliError::Io(
                        "terminal stdin reader panicked during shutdown".to_owned(),
                    ))
                }
            };
            match (wake, joined) {
                (_, Err(error)) => Err(error),
                (Err(error), Ok(())) => Err(error),
                (Ok(()), Ok(())) => Ok(()),
            }
        }
    }

    impl Drop for StdinPump {
        fn drop(&mut self) {
            let _ = self.shutdown();
        }
    }

    fn stdin_reader(
        input: OwnedFd,
        cancellation: OwnedFd,
        sender: mpsc::Sender<StdinEvent>,
        input_epoch: InputEpoch,
        #[cfg(test)] reader_test_seam: Option<StdinReaderTestSeam>,
    ) -> Result<(), String> {
        loop {
            let mut descriptors = [
                PollFd::new(&cancellation, PollFlags::IN),
                PollFd::new(&input, PollFlags::IN),
            ];
            match poll(&mut descriptors, None) {
                Ok(_) => {}
                Err(error) if error == rustix::io::Errno::INTR => continue,
                Err(error) => return Err(format!("poll terminal stdin: {error}")),
            }
            if !descriptors[0].revents().is_empty() {
                return Ok(());
            }

            let input_events = descriptors[1].revents();
            if input_events.contains(PollFlags::NVAL) {
                return Err("poll terminal stdin: descriptor became invalid".to_owned());
            }
            if !input_events.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR) {
                continue;
            }
            #[cfg(test)]
            if let Some(reader_test_seam) = &reader_test_seam {
                reader_test_seam.after_readable_poll()?;
            }
            let epoch = input_epoch.current();
            let mut buffer = [0_u8; STDIN_CHUNK_BYTES];
            match rustix::io::read(&input, &mut buffer) {
                Ok(0) => {
                    let _ = sender.blocking_send(StdinEvent::Eof);
                    return Ok(());
                }
                Ok(read) => {
                    if sender
                        .blocking_send(StdinEvent::Bytes {
                            epoch,
                            bytes: buffer[..read].to_vec(),
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                }
                Err(error) if error == rustix::io::Errno::INTR => {}
                Err(error) if error == rustix::io::Errno::AGAIN => {}
                Err(error) => {
                    let detail = error.to_string();
                    if sender
                        .blocking_send(StdinEvent::Error(detail.clone()))
                        .is_err()
                    {
                        return Ok(());
                    }
                    return Err(format!("read terminal stdin: {detail}"));
                }
            }
        }
    }

    #[derive(Clone, Eq, PartialEq)]
    enum PrefixAction {
        Input(Vec<u8>),
        Detach,
    }

    impl fmt::Debug for PrefixAction {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Input(bytes) => formatter
                    .debug_struct("Input")
                    .field("byte_len", &bytes.len())
                    .finish(),
                Self::Detach => formatter.write_str("Detach"),
            }
        }
    }

    struct PrefixParser {
        prefix: Option<u8>,
        pending_deadline: Option<Instant>,
        detached: bool,
    }

    impl PrefixParser {
        const fn new(prefix: Option<u8>) -> Self {
            Self {
                prefix,
                pending_deadline: None,
                detached: false,
            }
        }

        fn feed(&mut self, bytes: &[u8], now: Instant) -> Vec<PrefixAction> {
            if self.detached || bytes.is_empty() {
                return Vec::new();
            }
            let Some(prefix) = self.prefix else {
                return vec![PrefixAction::Input(bytes.to_vec())];
            };
            let mut actions = Vec::new();
            let mut ordinary =
                Vec::with_capacity(bytes.len() + usize::from(self.pending_deadline.is_some()));
            if self
                .pending_deadline
                .is_some_and(|deadline| now >= deadline)
            {
                self.pending_deadline = None;
                ordinary.push(prefix);
            }
            for &byte in bytes {
                if self.pending_deadline.take().is_some() {
                    if byte == b'.' {
                        if !ordinary.is_empty() {
                            actions.push(PrefixAction::Input(std::mem::take(&mut ordinary)));
                        }
                        actions.push(PrefixAction::Detach);
                        self.detached = true;
                        break;
                    }
                    ordinary.push(prefix);
                    if byte != prefix {
                        ordinary.push(byte);
                    }
                } else if byte == prefix {
                    self.pending_deadline = Some(now + CONTROL_PREFIX_TIMEOUT);
                } else {
                    ordinary.push(byte);
                }
            }
            if !ordinary.is_empty() {
                actions.push(PrefixAction::Input(ordinary));
            }
            actions
        }

        const fn deadline(&self) -> Option<Instant> {
            self.pending_deadline
        }

        fn flush_pending(&mut self) -> Option<Vec<u8>> {
            self.pending_deadline
                .take()
                .and(self.prefix)
                .map(|prefix| vec![prefix])
        }

        fn clear_pending(&mut self) {
            self.pending_deadline = None;
        }

        const fn detached(&self) -> bool {
            self.detached
        }
    }

    fn take_pending_active_input(
        prefix: &mut PrefixParser,
        state: TerminalViewTransportState,
    ) -> Option<Vec<u8>> {
        let pending = prefix.flush_pending();
        if state == TerminalViewTransportState::Active {
            pending
        } else {
            None
        }
    }

    async fn wait_for_prefix_deadline(deadline: Option<Instant>) {
        let Some(deadline) = deadline else {
            std::future::pending::<()>().await;
            return;
        };
        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
    }

    async fn wait_for_viewport_deadline(deadline: Option<Instant>) {
        let Some(deadline) = deadline else {
            std::future::pending::<()>().await;
            return;
        };
        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
    }

    struct ResizeCoalescer {
        pending: Option<TerminalSize>,
        last_submitted: TerminalSize,
    }

    impl ResizeCoalescer {
        const fn new(initial_size: TerminalSize) -> Self {
            Self {
                pending: None,
                last_submitted: initial_size,
            }
        }

        fn observe(
            &mut self,
            size: TerminalSize,
            state: TerminalViewTransportState,
        ) -> Option<TerminalSize> {
            if state == TerminalViewTransportState::Active {
                self.pending = None;
                self.submit_if_changed(size)
            } else {
                self.pending = Some(size);
                None
            }
        }

        fn enter_transport_state(
            &mut self,
            state: TerminalViewTransportState,
        ) -> (TerminalViewTransportState, Option<TerminalSize>) {
            let pending = (state == TerminalViewTransportState::Active)
                .then(|| self.pending.take())
                .flatten()
                .and_then(|size| self.submit_if_changed(size));
            let effective = if pending.is_some() {
                TerminalViewTransportState::Synchronizing
            } else {
                state
            };
            (effective, pending)
        }

        fn submit_if_changed(&mut self, size: TerminalSize) -> Option<TerminalSize> {
            if size == self.last_submitted {
                return None;
            }
            self.last_submitted = size;
            Some(size)
        }
    }

    struct StatusRenderer {
        device: Option<String>,
        physical_size: TerminalSize,
        path: TerminalViewConnectionPath,
        rtt_ms: Option<u32>,
    }

    impl StatusRenderer {
        fn new(device: Option<String>, physical_size: TerminalSize) -> Self {
            Self {
                device,
                physical_size,
                path: TerminalViewConnectionPath::Unknown,
                rtt_ms: None,
            }
        }

        fn enabled(&self) -> bool {
            self.device.is_some() && self.physical_size.rows > 1
        }

        fn is_remote(&self) -> bool {
            self.device.is_some()
        }

        fn resize(&mut self, physical_size: TerminalSize) {
            self.physical_size = physical_size;
        }

        fn observe(&mut self, status: TerminalViewConnectionStatus) -> Result<(), CliError> {
            if self.device.as_deref() != Some(status.device()) {
                return Err(terminal_daemon_error(
                    DomainErrorKind::MalformedFrame,
                    "terminal connection status changed its frozen device alias",
                ));
            }
            self.path = status.path();
            self.rtt_ms = status.rtt_ms();
            Ok(())
        }

        fn reset_for_reconnect(&mut self) {
            // A replacement stream has a new path-observation epoch. Do not
            // let an old direct/relay sample reappear while it synchronizes.
            self.path = TerminalViewConnectionPath::Unknown;
            self.rtt_ms = None;
        }

        fn composed_text(&self, transport_state: TerminalViewTransportState) -> Option<String> {
            let device = self.device.as_deref().filter(|_| self.enabled())?;
            let (path, latency) = if transport_state != TerminalViewTransportState::Reconnecting {
                match self.path {
                    TerminalViewConnectionPath::Direct => {
                        ("direct", self.rtt_ms.map(|rtt| format!("{rtt} ms")))
                    }
                    TerminalViewConnectionPath::Relay => {
                        ("relay", self.rtt_ms.map(|rtt| format!("{rtt} ms")))
                    }
                    TerminalViewConnectionPath::Unknown => ("--", None),
                }
            } else {
                ("--", None)
            };
            Some(format!(
                "{device} | {path} | {}",
                latency.as_deref().unwrap_or("--")
            ))
        }
    }

    enum ViewportEffect {
        None,
        Render,
        RequestHistoryWindow(TerminalHistoryWindowQuery),
        RenderAndRequestHistoryWindow(TerminalHistoryWindowQuery),
        Resume,
    }

    #[derive(Default)]
    struct ViewportPresentationPacer {
        last_presented: Option<Instant>,
        dirty: bool,
        pending_deadline: Option<Instant>,
    }

    impl ViewportPresentationPacer {
        fn mark_dirty(&mut self, now: Instant) {
            if self.dirty {
                return;
            }
            self.dirty = true;
            let earliest = self
                .last_presented
                .and_then(|last| last.checked_add(MIN_VIEWPORT_PRESENT_INTERVAL));
            self.pending_deadline =
                Some(earliest.filter(|deadline| *deadline > now).unwrap_or(now));
        }

        const fn deadline(&self) -> Option<Instant> {
            self.pending_deadline
        }

        fn due(&self, now: Instant) -> bool {
            self.dirty
                && self
                    .pending_deadline
                    .is_some_and(|deadline| now >= deadline)
        }

        fn mark_presented(&mut self, now: Instant) {
            self.last_presented = Some(now);
            self.dirty = false;
            self.pending_deadline = None;
        }

        fn cancel(&mut self) {
            self.dirty = false;
            self.pending_deadline = None;
        }
    }

    struct HistoryViewport {
        notice: Option<&'static str>,
    }

    enum ViewportState {
        Live,
        History(HistoryViewport),
        ResumePending {
            retained_input: Vec<u8>,
            snapshot_applied: bool,
            presented_scroll_metrics: Option<TerminalScrollMetrics>,
        },
    }

    struct ViewportController {
        state: ViewportState,
        content_size: TerminalSize,
        live_metrics: Option<TerminalScrollMetrics>,
        gutter_column: Option<u16>,
        drag_grab_row: Option<u16>,
        drag_last_request: Option<Instant>,
        drag_deferred_target: Option<u64>,
        // Seeded for the initial live frame; unlike the cache's latest locally
        // presentable target, it subsequently advances only after a complete
        // outer-terminal transaction succeeds.
        last_presented_scroll_metrics: Option<TerminalScrollMetrics>,
        // This is the physical column committed by the same successful outer
        // transaction, not merely the most recently requested layout.
        presented_gutter_column: Option<u16>,
        discard_window_response: bool,
        window_cache: ViewportCache<TerminalSurfaceRow>,
    }

    #[derive(Clone, Copy)]
    struct ViewportDeltaPlan {
        content_size: TerminalSize,
        gutter_column: Option<u16>,
        live_metrics: Option<TerminalScrollMetrics>,
        update_live_metrics: bool,
        content_size_changed: bool,
        anchor_observation: Option<ViewportAnchorObservation>,
        selection_source: Option<SelectionSourceIdentity>,
    }

    impl ViewportController {
        fn with_layout(layout: ChromeLayout, live_metrics: Option<TerminalScrollMetrics>) -> Self {
            let mut controller = Self {
                state: ViewportState::Live,
                content_size: layout.child,
                live_metrics,
                gutter_column: layout.gutter_column,
                drag_grab_row: None,
                drag_last_request: None,
                drag_deferred_target: None,
                last_presented_scroll_metrics: live_metrics,
                presented_gutter_column: None,
                discard_window_response: false,
                window_cache: ViewportCache::new(),
            };
            controller.observe_window_anchor(live_metrics);
            controller
        }

        const fn content_size(&self) -> TerminalSize {
            self.content_size
        }

        const fn is_live(&self) -> bool {
            matches!(&self.state, ViewportState::Live)
        }

        const fn is_history(&self) -> bool {
            matches!(&self.state, ViewportState::History(_))
        }

        fn has_complete_cached_viewport(&self) -> bool {
            self.is_history() && self.window_cache.visible_rows().is_some()
        }

        fn has_complete_presentable_view(&self) -> bool {
            self.is_live() || self.has_complete_cached_viewport()
        }

        const fn is_resume_pending(&self) -> bool {
            matches!(&self.state, ViewportState::ResumePending { .. })
        }

        fn resize(&mut self, content_size: TerminalSize) {
            let changed = self.content_size != content_size;
            self.content_size = content_size;
            if changed {
                self.last_presented_scroll_metrics = None;
                if let ViewportState::ResumePending {
                    presented_scroll_metrics,
                    ..
                } = &mut self.state
                {
                    // The old thumb geometry belongs to the previous viewport.
                    // Wait for the replacement snapshot instead of projecting it
                    // onto a size it never described.
                    *presented_scroll_metrics = None;
                }
                self.observe_window_anchor(self.live_metrics);
            }
        }

        fn set_layout(&mut self, layout: ChromeLayout) {
            self.resize(layout.child);
            self.gutter_column = layout.gutter_column;
            if self.gutter_column.is_none() {
                self.drag_grab_row = None;
                self.drag_last_request = None;
                self.drag_deferred_target = None;
            }
        }

        fn observe_window_anchor(&mut self, metrics: Option<TerminalScrollMetrics>) {
            let Some(metrics) = metrics.filter(|metrics| metrics.is_valid()) else {
                return;
            };
            let _ = self
                .window_cache
                .observe_anchor(TerminalHistoryWindowAnchor {
                    epoch: metrics.epoch,
                    revision: metrics.revision,
                    max_offset_from_bottom: metrics.max_offset_from_bottom,
                    viewport: self.content_size,
                });
        }

        fn prefetch_live(&mut self) -> Option<TerminalHistoryWindowQuery> {
            if !self.is_live()
                || self
                    .live_metrics
                    .is_none_or(|metrics| metrics.max_offset_from_bottom == 0)
            {
                return None;
            }
            self.window_cache.set_target(0).request
        }

        fn refetch_history_window(&mut self) -> Option<TerminalHistoryWindowQuery> {
            if !self.is_history() || self.window_cache.visible_rows().is_some() {
                return None;
            }
            self.window_cache
                .set_target(self.window_cache.desired_offset_from_bottom())
                .request
        }

        fn effective_screen(&self, authoritative: ActiveScreen) -> ActiveScreen {
            if self.is_history() {
                ActiveScreen::Main
            } else {
                authoritative
            }
        }

        fn outside_layout(&self, mouse: &SgrMouse) -> bool {
            mouse.row > self.content_size.rows
                || mouse.column > self.gutter_column.unwrap_or(self.content_size.columns)
        }

        const fn gutter_drag_active(&self) -> bool {
            self.drag_grab_row.is_some()
        }

        fn gutter_hit(&self, mouse: &SgrMouse) -> bool {
            Some(mouse.column) == self.gutter_column && mouse.row <= self.content_size.rows
        }

        fn handle_gutter_mouse(
            &mut self,
            mouse: &SgrMouse,
            allow_live_navigation: bool,
        ) -> Option<ViewportEffect> {
            self.handle_gutter_mouse_at(mouse, allow_live_navigation, Instant::now())
        }

        fn handle_gutter_mouse_at(
            &mut self,
            mouse: &SgrMouse,
            allow_live_navigation: bool,
            now: Instant,
        ) -> Option<ViewportEffect> {
            if let Some(grab) = self.drag_grab_row {
                if mouse.release {
                    self.drag_grab_row = None;
                    self.drag_last_request = None;
                    if let Some(target) = self.drag_deferred_target.take() {
                        self.window_cache.defer_pending_request();
                        return Some(self.scroll_to_offset(target));
                    }
                    if self.is_history() && self.window_cache.desired_offset_from_bottom() == 0 {
                        return Some(self.start_resume(Vec::new()));
                    }
                    return Some(ViewportEffect::None);
                }
                if mouse.is_motion() {
                    if self.is_live() && !allow_live_navigation {
                        return Some(ViewportEffect::None);
                    }
                    let Some(metrics) = self.scroll_metrics() else {
                        self.drag_grab_row = None;
                        return Some(ViewportEffect::None);
                    };
                    let Some(geometry) = ScrollbarGeometry::new(self.content_size.rows, metrics)
                    else {
                        self.drag_grab_row = None;
                        return Some(ViewportEffect::None);
                    };
                    let row = mouse.row.clamp(1, self.content_size.rows);
                    let target =
                        geometry.offset_for_pointer(row, grab, metrics.max_offset_from_bottom);
                    if self.is_history() && target == self.window_cache.desired_offset_from_bottom()
                    {
                        return Some(ViewportEffect::None);
                    }
                    let effect = self.scroll_to_offset(target);
                    return Some(self.pace_drag_effect(effect, target, now));
                }
                self.drag_grab_row = None;
                self.drag_last_request = None;
                self.drag_deferred_target = None;
            }
            if Some(mouse.column) != self.gutter_column || mouse.row > self.content_size.rows {
                return None;
            }
            if self.is_live() && !allow_live_navigation {
                return Some(ViewportEffect::None);
            }
            if mouse.is_wheel() {
                return Some(self.navigate(mouse.wheel_is_up(), 1));
            }
            if mouse.release {
                self.drag_grab_row = None;
                self.drag_last_request = None;
                self.drag_deferred_target = None;
                return Some(ViewportEffect::None);
            }
            let Some(metrics) = self.scroll_metrics() else {
                return Some(ViewportEffect::None);
            };
            let Some(geometry) = ScrollbarGeometry::new(self.content_size.rows, metrics) else {
                return Some(ViewportEffect::None);
            };
            if mouse.is_left_press() {
                let grab = if geometry.contains_thumb(mouse.row) {
                    geometry.grab_row(mouse.row)
                } else {
                    geometry.thumb_len / 2
                };
                self.drag_grab_row = Some(grab);
                let target =
                    geometry.offset_for_pointer(mouse.row, grab, metrics.max_offset_from_bottom);
                let effect = self.scroll_to_offset(target);
                if matches!(
                    &effect,
                    ViewportEffect::RequestHistoryWindow(_)
                        | ViewportEffect::RenderAndRequestHistoryWindow(_)
                ) {
                    self.drag_last_request = Some(now);
                }
                return Some(effect);
            }
            Some(ViewportEffect::None)
        }

        fn pace_drag_effect(
            &mut self,
            effect: ViewportEffect,
            target: u64,
            now: Instant,
        ) -> ViewportEffect {
            let is_window_request = matches!(
                &effect,
                ViewportEffect::RequestHistoryWindow(_)
                    | ViewportEffect::RenderAndRequestHistoryWindow(_)
            );
            if !is_window_request {
                return effect;
            }
            if self
                .drag_last_request
                .is_some_and(|last| now.saturating_duration_since(last) < DRAG_REQUEST_INTERVAL)
            {
                self.window_cache.defer_pending_request();
                self.drag_deferred_target = Some(target);
                return if matches!(&effect, ViewportEffect::RenderAndRequestHistoryWindow(_)) {
                    ViewportEffect::Render
                } else {
                    ViewportEffect::None
                };
            }
            self.drag_last_request = Some(now);
            self.drag_deferred_target = None;
            effect
        }

        fn navigate(&mut self, older: bool, amount: usize) -> ViewportEffect {
            if matches!(self.state, ViewportState::Live) {
                if !older {
                    return ViewportEffect::None;
                }
                if self
                    .live_metrics
                    .is_some_and(|metrics| metrics.max_offset_from_bottom == 0)
                {
                    return ViewportEffect::None;
                }
                let Some(anchor) = self.window_cache.anchor() else {
                    return ViewportEffect::None;
                };
                let target = u64::try_from(amount)
                    .unwrap_or(u64::MAX)
                    .min(anchor.max_offset_from_bottom);
                let update = self.window_cache.set_target(target);
                self.state = ViewportState::History(HistoryViewport { notice: None });
                return history_window_effect(update);
            }
            let ViewportState::History(_) = &self.state else {
                return ViewportEffect::None;
            };
            let current = self.window_cache.desired_offset_from_bottom();
            let amount = u64::try_from(amount).unwrap_or(u64::MAX);
            let maximum = self
                .window_cache
                .anchor()
                .map_or(current, |anchor| anchor.max_offset_from_bottom);
            let target = if older {
                current.saturating_add(amount).min(maximum)
            } else {
                current.saturating_sub(amount)
            };
            if target == 0 {
                return self.start_resume(Vec::new());
            }
            history_window_effect(self.window_cache.set_target(target))
        }

        fn apply_view_history_window(
            &mut self,
            result: TerminalViewHistoryWindow,
        ) -> Result<ViewportEffect, CliError> {
            if self.discard_window_response {
                self.discard_window_response = false;
                return Ok(ViewportEffect::None);
            }
            if !self.window_cache.request_pending() {
                return Err(terminal_daemon_error(
                    DomainErrorKind::MalformedFrame,
                    "terminal history window arrived without a pending request",
                ));
            }
            match result {
                TerminalSurfaceHistoryWindowResult::Frame(frame) => {
                    if frame.anchor.viewport != self.content_size {
                        self.window_cache.defer_pending_request();
                        let update = self
                            .window_cache
                            .set_target(self.window_cache.desired_offset_from_bottom());
                        return Ok(history_window_effect(update));
                    }
                    let rebased = frame.disposition == TerminalViewportDisposition::Rebased;
                    let installed = self
                        .window_cache
                        .install_window(CachedViewportWindow {
                            disposition: frame.disposition,
                            anchor: frame.anchor,
                            target_offset_from_bottom: frame.target_offset_from_bottom,
                            first_row_from_live_top: frame.first_row_from_live_top,
                            rows: frame.rows,
                        })
                        .map_err(|_| {
                            terminal_daemon_error(
                                DomainErrorKind::MalformedFrame,
                                "terminal history window failed cache validation",
                            )
                        })?;
                    if installed.render_local {
                        if self.window_cache.desired_offset_from_bottom() == 0 && self.is_live() {
                            return Ok(ViewportEffect::None);
                        }
                        if let ViewportState::History(history) = &mut self.state {
                            history.notice = rebased.then_some(
                                "[zterm: retained history changed; showing closest view]",
                            );
                        }
                    }
                    Ok(match (installed.render_local, installed.request) {
                        (true, Some(query)) => ViewportEffect::RenderAndRequestHistoryWindow(query),
                        (true, None) => ViewportEffect::Render,
                        (false, Some(query)) => ViewportEffect::RequestHistoryWindow(query),
                        (false, None) => ViewportEffect::None,
                    })
                }
                TerminalSurfaceHistoryWindowResult::HistoryChanged { .. }
                | TerminalSurfaceHistoryWindowResult::HistoryGap { .. } => {
                    self.window_cache.defer_pending_request();
                    self.window_cache.invalidate_rows();
                    // A content-free changed/gap response is not a complete
                    // replacement. Keep the last committed host frame intact;
                    // the next gesture may retry or normal input may resume.
                    Ok(ViewportEffect::None)
                }
            }
        }

        fn scroll_to_offset(&mut self, offset: u64) -> ViewportEffect {
            let ViewportState::History(_) = &self.state else {
                if self.live_metrics.is_none() || offset == 0 {
                    return ViewportEffect::None;
                }
                let Some(anchor) = self.window_cache.anchor() else {
                    return ViewportEffect::None;
                };
                let update = self
                    .window_cache
                    .set_target(offset.min(anchor.max_offset_from_bottom));
                self.state = ViewportState::History(HistoryViewport { notice: None });
                return history_window_effect(update);
            };
            if offset == 0 {
                if self.drag_grab_row.is_some() {
                    return history_window_effect(self.window_cache.set_target(0));
                }
                return self.start_resume(Vec::new());
            }
            history_window_effect(self.window_cache.set_target(offset))
        }

        fn scroll_metrics(&self) -> Option<TerminalScrollMetrics> {
            match &self.state {
                ViewportState::Live => self.live_metrics,
                ViewportState::History(_) => {
                    if self.window_cache.visible_rows().is_some() {
                        self.window_cache
                            .anchor()
                            .map(|anchor| TerminalScrollMetrics {
                                epoch: anchor.epoch,
                                revision: anchor.revision,
                                offset_from_bottom: self.window_cache.desired_offset_from_bottom(),
                                max_offset_from_bottom: anchor.max_offset_from_bottom,
                                viewport_rows: anchor.viewport.rows,
                            })
                    } else {
                        self.last_presented_scroll_metrics
                    }
                }
                ViewportState::ResumePending {
                    snapshot_applied,
                    presented_scroll_metrics,
                    ..
                } => {
                    if *snapshot_applied {
                        self.live_metrics
                            .filter(|metrics| metrics.viewport_rows == self.content_size.rows)
                    } else {
                        *presented_scroll_metrics
                    }
                }
            }
            .filter(|metrics| metrics.is_valid())
        }

        fn observe_presentation(&mut self) {
            if self.is_history() {
                let _ = self.window_cache.commit_visible_presentation();
            }
            self.last_presented_scroll_metrics = self.scroll_metrics();
            self.presented_gutter_column = self.gutter_column;
        }

        fn selection_source_identity(
            &self,
            surface: &AttachmentSurface,
        ) -> Option<SelectionSourceIdentity> {
            match &self.state {
                ViewportState::Live => Some(SelectionSourceIdentity::Live {
                    revision: surface.revision(),
                    screen: surface.active_screen(),
                    viewport: self.content_size,
                }),
                ViewportState::History(history) if history.notice.is_none() => self
                    .window_cache
                    .presented_slice_identity()
                    .map(SelectionSourceIdentity::History),
                ViewportState::History(_) | ViewportState::ResumePending { .. } => None,
            }
        }

        fn next_frame_selection_source_identity(
            &self,
            surface: &AttachmentSurface,
        ) -> Option<SelectionSourceIdentity> {
            match &self.state {
                ViewportState::Live => Some(SelectionSourceIdentity::Live {
                    revision: surface.revision(),
                    screen: surface.active_screen(),
                    viewport: self.content_size,
                }),
                ViewportState::History(history) if history.notice.is_none() => self
                    .window_cache
                    .visible_slice_identity()
                    .map(SelectionSourceIdentity::History),
                ViewportState::History(_) | ViewportState::ResumePending { .. } => None,
            }
        }

        fn selection_rows<'a>(
            &'a self,
            surface: &'a AttachmentSurface,
        ) -> Option<&'a [TerminalSurfaceRow]> {
            match &self.state {
                ViewportState::Live => Some(&surface.surface.rows),
                ViewportState::History(history) if history.notice.is_none() => {
                    self.window_cache.presented_rows()
                }
                ViewportState::History(_) | ViewportState::ResumePending { .. } => None,
            }
        }

        fn preview_delta(
            &self,
            candidate: &AttachmentSurface,
            metrics: Option<TerminalScrollMetrics>,
            live_layout: Option<ChromeLayout>,
        ) -> ViewportDeltaPlan {
            debug_assert_eq!(self.is_live(), live_layout.is_some());
            let content_size = live_layout.map_or(self.content_size, |layout| layout.child);
            let gutter_column =
                live_layout.map_or(self.gutter_column, |layout| layout.gutter_column);
            let update_live_metrics =
                metrics.is_some() || self.is_live() || self.is_resume_pending();
            let live_metrics = if update_live_metrics {
                metrics
            } else {
                self.live_metrics
            };
            let anchor_observation = metrics.filter(|metrics| metrics.is_valid()).map(|metrics| {
                self.window_cache
                    .preview_anchor_observation(TerminalHistoryWindowAnchor {
                        epoch: metrics.epoch,
                        revision: metrics.revision,
                        max_offset_from_bottom: metrics.max_offset_from_bottom,
                        viewport: content_size,
                    })
            });
            let history_identity = match anchor_observation {
                Some(observation) => observation.presented_slice_identity(),
                None => self.window_cache.presented_slice_identity(),
            };
            let selection_source = match &self.state {
                ViewportState::Live => Some(SelectionSourceIdentity::Live {
                    revision: candidate.revision(),
                    screen: candidate.active_screen(),
                    viewport: content_size,
                }),
                ViewportState::History(history) if history.notice.is_none() => {
                    history_identity.map(SelectionSourceIdentity::History)
                }
                ViewportState::History(_) | ViewportState::ResumePending { .. } => None,
            };
            ViewportDeltaPlan {
                content_size,
                gutter_column,
                live_metrics,
                update_live_metrics,
                content_size_changed: self.content_size != content_size,
                anchor_observation,
                selection_source,
            }
        }

        fn commit_delta(&mut self, plan: ViewportDeltaPlan) {
            if plan.update_live_metrics {
                self.live_metrics = plan.live_metrics;
            }
            if plan.content_size_changed {
                self.last_presented_scroll_metrics = None;
                if let ViewportState::ResumePending {
                    presented_scroll_metrics,
                    ..
                } = &mut self.state
                {
                    *presented_scroll_metrics = None;
                }
            }
            self.content_size = plan.content_size;
            self.gutter_column = plan.gutter_column;
            if self.gutter_column.is_none() {
                self.drag_grab_row = None;
                self.drag_last_request = None;
                self.drag_deferred_target = None;
            }
            if let Some(observation) = plan.anchor_observation {
                let _ = self.window_cache.commit_anchor_observation(observation);
            }
        }

        fn retain_or_resume(&mut self, bytes: Vec<u8>) -> Result<ViewportEffect, CliError> {
            match &mut self.state {
                ViewportState::Live => Ok(ViewportEffect::None),
                ViewportState::History(_) => self.begin_resume(bytes),
                ViewportState::ResumePending { retained_input, .. } => {
                    append_resume_input(retained_input, &bytes)?;
                    Ok(ViewportEffect::None)
                }
            }
        }

        fn begin_resume(&mut self, bytes: Vec<u8>) -> Result<ViewportEffect, CliError> {
            match &mut self.state {
                ViewportState::ResumePending { retained_input, .. } => {
                    append_resume_input(retained_input, &bytes)?;
                    Ok(ViewportEffect::None)
                }
                ViewportState::Live | ViewportState::History(_) => {
                    if bytes.len() > RESUME_INPUT_BOUND {
                        return Err(resume_input_overflow());
                    }
                    Ok(self.start_resume(bytes))
                }
            }
        }

        fn start_resume(&mut self, retained_input: Vec<u8>) -> ViewportEffect {
            let presented_scroll_metrics = self.last_presented_scroll_metrics;
            self.discard_window_response |= self.window_cache.request_pending();
            self.window_cache.invalidate();
            self.drag_grab_row = None;
            self.drag_last_request = None;
            self.drag_deferred_target = None;
            self.state = ViewportState::ResumePending {
                retained_input,
                snapshot_applied: false,
                presented_scroll_metrics,
            };
            ViewportEffect::Resume
        }

        fn retain_resume_input(&mut self, bytes: &[u8]) -> Result<(), CliError> {
            let ViewportState::ResumePending { retained_input, .. } = &mut self.state else {
                return Ok(());
            };
            append_resume_input(retained_input, bytes)
        }

        fn observe_snapshot(&mut self, metrics: Option<TerminalScrollMetrics>) {
            self.live_metrics = metrics;
            self.observe_window_anchor(metrics);
            match &mut self.state {
                ViewportState::Live | ViewportState::History(_) => {}
                ViewportState::ResumePending {
                    snapshot_applied, ..
                } => *snapshot_applied = true,
            }
        }

        fn observe_sync_required(&mut self) {
            if self.is_live() {
                let _ = self.start_resume(Vec::new());
            }
        }

        fn reset_presentation_for_reconnect(&mut self) {
            self.drag_grab_row = None;
            self.drag_last_request = None;
            self.drag_deferred_target = None;
            if self.is_history() {
                let _ = self.start_resume(Vec::new());
            } else {
                self.discard_window_response |= self.window_cache.request_pending();
                self.window_cache.invalidate();
            }
            // A replacement attachment epoch cannot authenticate even the
            // previously live extent. Keep the old pixels in place, but do
            // not project their metrics into reconnecting chrome.
            self.live_metrics = None;
            self.last_presented_scroll_metrics = None;
            if let ViewportState::ResumePending {
                snapshot_applied,
                presented_scroll_metrics,
                ..
            } = &mut self.state
            {
                // A transport reconnect has no validated relationship to the
                // pixels from the previous attachment epoch.
                *snapshot_applied = false;
                *presented_scroll_metrics = None;
            }
        }

        fn finish_resume(&mut self) -> Option<Vec<u8>> {
            let ViewportState::ResumePending {
                snapshot_applied: true,
                ..
            } = &self.state
            else {
                return None;
            };
            let ViewportState::ResumePending { retained_input, .. } =
                std::mem::replace(&mut self.state, ViewportState::Live)
            else {
                unreachable!();
            };
            Some(retained_input)
        }

        fn visible_semantic_history_rows(
            &self,
        ) -> Option<(Vec<&TerminalSurfaceRow>, usize, Option<&'static str>)> {
            let ViewportState::History(history) = &self.state else {
                return None;
            };
            let rows = self.window_cache.visible_rows()?;
            let rows = rows.iter().collect();
            Some((rows, usize::from(self.content_size.rows), history.notice))
        }
    }

    fn history_window_effect(update: ViewportCacheUpdate) -> ViewportEffect {
        match (update.render_local, update.request) {
            (true, Some(query)) => ViewportEffect::RenderAndRequestHistoryWindow(query),
            (true, None) => ViewportEffect::Render,
            (false, Some(query)) => ViewportEffect::RequestHistoryWindow(query),
            (false, None) => ViewportEffect::None,
        }
    }

    fn append_resume_input(retained: &mut Vec<u8>, bytes: &[u8]) -> Result<(), CliError> {
        let combined = retained
            .len()
            .checked_add(bytes.len())
            .ok_or_else(resume_input_overflow)?;
        if combined > RESUME_INPUT_BOUND {
            return Err(resume_input_overflow());
        }
        retained.extend_from_slice(bytes);
        Ok(())
    }

    fn resume_input_overflow() -> CliError {
        terminal_daemon_error(
            DomainErrorKind::ResourceExhausted,
            "terminal input retained while returning from history exceeded its fixed bound",
        )
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum HostInputEvent {
        Bytes(Vec<u8>),
        LegacyCtrlC,
        EnhancedKey(EnhancedKey),
        Paste(Vec<u8>),
        PageUp,
        PageDown,
        Mouse(SgrMouse),
    }

    struct HostInputCodec {
        pending: Vec<u8>,
        in_paste: bool,
    }

    impl HostInputCodec {
        const fn new() -> Self {
            Self {
                pending: Vec::new(),
                in_paste: false,
            }
        }

        fn feed(&mut self, bytes: &[u8]) -> Result<Vec<HostInputEvent>, CliError> {
            self.pending.extend_from_slice(bytes);
            let mut events = Vec::new();
            loop {
                if self.pending.is_empty() {
                    break;
                }
                if self.in_paste {
                    if let Some(index) = find_bytes(&self.pending, PASTE_END) {
                        let end = index + PASTE_END.len();
                        if end > RESUME_INPUT_BOUND {
                            return Err(paste_input_overflow());
                        }
                        events.push(HostInputEvent::Paste(self.pending.drain(..end).collect()));
                        self.in_paste = false;
                        continue;
                    }
                    if self.pending.len() > RESUME_INPUT_BOUND {
                        return Err(paste_input_overflow());
                    }
                    break;
                }
                if self.pending.starts_with(PASTE_START) {
                    self.in_paste = true;
                    continue;
                }
                if self.pending.starts_with(PAGE_UP) {
                    self.pending.drain(..PAGE_UP.len());
                    events.push(HostInputEvent::PageUp);
                    continue;
                }
                if self.pending.starts_with(PAGE_DOWN) {
                    self.pending.drain(..PAGE_DOWN.len());
                    events.push(HostInputEvent::PageDown);
                    continue;
                }
                if self.pending.starts_with(b"\x1b[<") {
                    if let Some(end) = self
                        .pending
                        .iter()
                        .position(|byte| matches!(byte, b'M' | b'm'))
                    {
                        let length = end + 1;
                        let raw: Vec<u8> = self.pending.drain(..length).collect();
                        if let Some(mouse) = SgrMouse::parse(raw.clone()) {
                            events.push(HostInputEvent::Mouse(mouse));
                        } else {
                            push_raw_host_bytes(&mut events, raw);
                        }
                        continue;
                    }
                    if self.pending.len() < HOST_SEQUENCE_BOUND {
                        break;
                    }
                }
                if self.pending.starts_with(b"\x1b[") {
                    if let Some(end) = self
                        .pending
                        .iter()
                        .enumerate()
                        .skip(2)
                        .find_map(|(index, byte)| (0x40..=0x7e).contains(byte).then_some(index))
                    {
                        let raw: Vec<u8> = self.pending.drain(..=end).collect();
                        if let Some(key) = EnhancedKey::parse(raw.clone()) {
                            events.push(HostInputEvent::EnhancedKey(key));
                        } else {
                            push_raw_host_bytes(&mut events, raw);
                        }
                        continue;
                    }
                    if self.pending.len() < HOST_SEQUENCE_BOUND {
                        break;
                    }
                }
                if known_host_prefix(&self.pending) {
                    break;
                }
                push_host_bytes(&mut events, vec![self.pending.remove(0)]);
            }
            Ok(events)
        }

        const fn paste_in_progress(&self) -> bool {
            self.in_paste
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct SgrMouse {
        code: u16,
        column: u16,
        row: u16,
        release: bool,
        raw: Vec<u8>,
    }

    impl SgrMouse {
        fn parse(raw: Vec<u8>) -> Option<Self> {
            let release = *raw.last()? == b'm';
            let body = raw
                .strip_prefix(b"\x1b[<")?
                .get(..raw.len().checked_sub(4)?)?;
            let mut fields = body.split(|byte| *byte == b';');
            let code = parse_decimal(fields.next()?)?;
            let column = parse_decimal(fields.next()?)?;
            let row = parse_decimal(fields.next()?)?;
            if fields.next().is_some() || column == 0 || row == 0 {
                return None;
            }
            Some(Self {
                code,
                column,
                row,
                release,
                raw,
            })
        }

        const fn is_wheel(&self) -> bool {
            self.code & 64 != 0
        }

        const fn wheel_is_up(&self) -> bool {
            self.code & 1 == 0
        }

        const fn is_motion(&self) -> bool {
            self.code & 32 != 0
        }

        const fn is_left_press(&self) -> bool {
            !self.release && !self.is_motion() && self.code & 3 == 0
        }

        const fn is_unmodified(&self) -> bool {
            self.code & (4 | 8 | 16) == 0
        }

        fn content_point(&self, viewport: TerminalSize) -> Option<TerminalTextPoint> {
            if self.row == 0
                || self.column == 0
                || self.row > viewport.rows
                || self.column > viewport.columns
            {
                return None;
            }
            Some(TerminalTextPoint::new(self.row - 1, self.column - 1))
        }
    }

    fn parse_decimal(bytes: &[u8]) -> Option<u16> {
        if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let mut value = 0_u16;
        for byte in bytes {
            value = value
                .checked_mul(10)?
                .checked_add(u16::from(*byte - b'0'))?;
        }
        Some(value)
    }

    fn known_host_prefix(bytes: &[u8]) -> bool {
        [PAGE_UP, PAGE_DOWN, PASTE_START]
            .into_iter()
            .any(|sequence| sequence.starts_with(bytes))
            || b"\x1b[<".starts_with(bytes)
    }

    fn find_bytes(bytes: &[u8], needle: &[u8]) -> Option<usize> {
        bytes
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn push_host_bytes(events: &mut Vec<HostInputEvent>, bytes: Vec<u8>) {
        for (index, span) in bytes.split(|byte| *byte == 0x03).enumerate() {
            if index > 0 {
                events.push(HostInputEvent::LegacyCtrlC);
            }
            if span.is_empty() {
                continue;
            }
            push_raw_host_bytes(events, span.to_vec());
        }
    }

    fn push_raw_host_bytes(events: &mut Vec<HostInputEvent>, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        if let Some(HostInputEvent::Bytes(previous)) = events.last_mut() {
            previous.extend_from_slice(&bytes);
        } else {
            events.push(HostInputEvent::Bytes(bytes));
        }
    }

    fn paste_input_overflow() -> CliError {
        terminal_daemon_error(
            DomainErrorKind::ResourceExhausted,
            "bracketed terminal paste exceeded its fixed input bound",
        )
    }

    fn history_owns_gestures(active_screen: ActiveScreen, modes: TerminalModes) -> bool {
        active_screen == ActiveScreen::Main && modes.mouse_mode == TerminalMouseMode::None
    }

    enum PointerRoute {
        Viewport(ViewportEffect),
        Child(Vec<u8>),
        SelectionChanged,
        Ignore,
    }

    fn route_pointer(
        mouse: &SgrMouse,
        viewport: &mut ViewportController,
        surface: &AttachmentSurface,
        selection: &mut SelectionController,
        allow_live_navigation: bool,
    ) -> Result<PointerRoute, CliError> {
        if viewport.gutter_drag_active() {
            selection.clear();
            return Ok(PointerRoute::Viewport(
                viewport
                    .handle_gutter_mouse(mouse, allow_live_navigation)
                    .unwrap_or(ViewportEffect::None),
            ));
        }

        if selection.owns_pointer_sequence() {
            if mouse.release {
                selection.finish();
                return Ok(PointerRoute::SelectionChanged);
            }
            if mouse.is_motion() {
                let source = viewport.selection_source_identity(surface);
                let point = mouse.content_point(viewport.content_size());
                let rows = viewport.selection_rows(surface);
                match (source, point, rows) {
                    (Some(source), Some(point), Some(rows)) => selection
                        .update(source, point, rows)
                        .map_err(selection_error)?,
                    _ => selection.cancel(),
                }
                return Ok(PointerRoute::SelectionChanged);
            }
            return Ok(PointerRoute::Ignore);
        }

        if viewport.gutter_hit(mouse) {
            selection.clear();
            return Ok(PointerRoute::Viewport(
                viewport
                    .handle_gutter_mouse(mouse, allow_live_navigation)
                    .unwrap_or(ViewportEffect::None),
            ));
        }
        if viewport.outside_layout(mouse) {
            return Ok(PointerRoute::Ignore);
        }

        if viewport.is_history() {
            if mouse.is_wheel() {
                selection.clear();
                return Ok(PointerRoute::Viewport(
                    viewport.navigate(mouse.wheel_is_up(), 1),
                ));
            }
            if mouse.is_left_press() && mouse.is_unmodified() {
                return begin_pointer_selection(mouse, viewport, surface, selection);
            }
            return Ok(PointerRoute::Ignore);
        }

        let modes = surface.modes();
        if modes.mouse_mode != TerminalMouseMode::None {
            selection.clear();
            return Ok(route_mouse_to_child(mouse, surface.active_screen(), modes)
                .map(PointerRoute::Child)
                .unwrap_or(PointerRoute::Ignore));
        }
        if mouse.is_wheel()
            && surface.active_screen() == ActiveScreen::Alternate
            && modes.alternate_scroll
        {
            return Ok(PointerRoute::Child(emulated_wheel_cursor_keys(
                mouse.wheel_is_up(),
                modes.application_cursor,
            )));
        }
        if mouse.is_wheel()
            && allow_live_navigation
            && history_owns_gestures(surface.active_screen(), modes)
        {
            selection.clear();
            return Ok(PointerRoute::Viewport(
                viewport.navigate(mouse.wheel_is_up(), 1),
            ));
        }
        if mouse.is_left_press() && mouse.is_unmodified() {
            return begin_pointer_selection(mouse, viewport, surface, selection);
        }
        Ok(PointerRoute::Ignore)
    }

    fn begin_pointer_selection(
        mouse: &SgrMouse,
        viewport: &ViewportController,
        surface: &AttachmentSurface,
        selection: &mut SelectionController,
    ) -> Result<PointerRoute, CliError> {
        let Some(source) = viewport.selection_source_identity(surface) else {
            selection.clear();
            return Ok(PointerRoute::Ignore);
        };
        let Some(rows) = viewport.selection_rows(surface) else {
            selection.clear();
            return Ok(PointerRoute::Ignore);
        };
        let Some(point) = mouse.content_point(viewport.content_size()) else {
            return Ok(PointerRoute::Ignore);
        };
        selection
            .begin(source, point, rows)
            .map_err(selection_error)?;
        Ok(PointerRoute::SelectionChanged)
    }

    fn selection_error(error: TerminalTextSelectionError) -> CliError {
        let kind = match error {
            TerminalTextSelectionError::Clipboard(_) => DomainErrorKind::ResourceExhausted,
            TerminalTextSelectionError::InvalidRange
            | TerminalTextSelectionError::InvalidSurface => DomainErrorKind::MalformedFrame,
        };
        terminal_daemon_error(kind, "terminal selection could not be projected")
    }

    const fn live_history_navigation_allowed(state: TerminalViewTransportState) -> bool {
        matches!(state, TerminalViewTransportState::Active)
    }

    fn route_mouse_to_child(
        mouse: &SgrMouse,
        active_screen: ActiveScreen,
        modes: TerminalModes,
    ) -> Option<Vec<u8>> {
        if modes.mouse_mode != TerminalMouseMode::None {
            return mouse_event_allowed(mouse, modes.mouse_mode)
                .then(|| encode_child_mouse(mouse, modes.mouse_encoding))
                .flatten();
        }
        if mouse.is_wheel() && active_screen == ActiveScreen::Alternate && modes.alternate_scroll {
            return Some(emulated_wheel_cursor_keys(
                mouse.wheel_is_up(),
                modes.application_cursor,
            ));
        }
        None
    }

    fn mouse_event_allowed(mouse: &SgrMouse, mode: TerminalMouseMode) -> bool {
        if mouse.is_wheel() {
            return true;
        }
        let motion = mouse.code & 32 != 0;
        if motion {
            return match mode {
                TerminalMouseMode::ButtonMotion => mouse.code & 3 != 3,
                TerminalMouseMode::AnyMotion => true,
                TerminalMouseMode::None
                | TerminalMouseMode::Press
                | TerminalMouseMode::PressRelease => false,
            };
        }
        if mouse.release {
            matches!(
                mode,
                TerminalMouseMode::PressRelease
                    | TerminalMouseMode::ButtonMotion
                    | TerminalMouseMode::AnyMotion
            )
        } else {
            true
        }
    }

    fn encode_child_mouse(mouse: &SgrMouse, encoding: TerminalMouseEncoding) -> Option<Vec<u8>> {
        if encoding == TerminalMouseEncoding::Sgr {
            return Some(mouse.raw.clone());
        }
        let code = if mouse.release {
            (mouse.code & !3) | 3
        } else {
            mouse.code
        };
        let values = [code, mouse.column, mouse.row];
        let mut bytes = b"\x1b[M".to_vec();
        match encoding {
            TerminalMouseEncoding::Default => {
                for value in values {
                    bytes.push(u8::try_from(value.checked_add(32)?).ok()?);
                }
            }
            TerminalMouseEncoding::Utf8 => {
                for value in values {
                    bytes.extend_from_slice(
                        char::from_u32(u32::from(value.checked_add(32)?))?
                            .encode_utf8(&mut [0; 4])
                            .as_bytes(),
                    );
                }
            }
            TerminalMouseEncoding::Sgr => unreachable!(),
        }
        Some(bytes)
    }

    fn emulated_wheel_cursor_keys(up: bool, application_cursor: bool) -> Vec<u8> {
        let sequence: &[u8] = match (up, application_cursor) {
            (true, true) => b"\x1bOA",
            (false, true) => b"\x1bOB",
            (true, false) => b"\x1b[A",
            (false, false) => b"\x1b[B",
        };
        sequence.to_vec()
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]

    enum DeltaRender {
        Applied,
        Gap,
    }

    enum KeyboardRoute {
        Copy,
        Consume,
        Forward {
            bytes: Vec<u8>,
            clear_selection: bool,
            reinterpret_legacy: bool,
        },
    }

    fn route_enhanced_input(
        key: &EnhancedKey,
        child_modes: TerminalModes,
        outer_flags: zterm_core::terminal::TerminalKeyboardFlags,
        selection_finalized: bool,
        lease: &mut CopyKeyLease,
    ) -> KeyboardRoute {
        if lease.consume(key) {
            return KeyboardRoute::Consume;
        }
        if selection_finalized && key.kind == KeyEventKind::Press && key.is_copy_shortcut() {
            lease.begin(key);
            return KeyboardRoute::Copy;
        }
        if outer_flags == child_modes.keyboard_flags {
            KeyboardRoute::Forward {
                bytes: key.raw.clone(),
                clear_selection: selection_finalized && key.kind != KeyEventKind::Release,
                reinterpret_legacy: false,
            }
        } else if selection_finalized
            && child_modes.keyboard_flags.is_empty()
            && outer_flags
                == keyboard::desired_outer_keyboard_flags(child_modes.keyboard_flags, true)
        {
            KeyboardRoute::Forward {
                bytes: key.legacy_bytes(child_modes),
                clear_selection: key.kind != KeyEventKind::Release,
                reinterpret_legacy: true,
            }
        } else {
            // Only the deliberate local-selection 0 -> flags-7 elevation has
            // enough information for a lossless legacy downgrade. Preserve
            // the original event for any other mismatch instead of inventing
            // child input semantics.
            KeyboardRoute::Forward {
                bytes: key.raw.clone(),
                clear_selection: selection_finalized && key.kind != KeyEventKind::Release,
                reinterpret_legacy: false,
            }
        }
    }

    fn host_events_from_legacy_bytes(bytes: Vec<u8>) -> Vec<HostInputEvent> {
        if bytes == PAGE_UP {
            return vec![HostInputEvent::PageUp];
        }
        if bytes == PAGE_DOWN {
            return vec![HostInputEvent::PageDown];
        }
        let mut events = Vec::new();
        push_host_bytes(&mut events, bytes);
        events
    }

    fn reconcile_presenter_selection(
        selection: &mut SelectionController,
        viewport: &ViewportController,
        surface: &AttachmentSurface,
        presenter: &mut DesktopPresenter,
    ) {
        let source = viewport.selection_source_identity(surface);
        selection.reconcile(source);
        presenter.set_selection(
            source,
            selection.range_for(source),
            selection.is_finalized(),
        );
    }

    fn reconcile_presenter_selection_for_next_frame(
        selection: &mut SelectionController,
        viewport: &ViewportController,
        surface: &AttachmentSurface,
        presenter: &mut DesktopPresenter,
    ) {
        let source = viewport.next_frame_selection_source_identity(surface);
        selection.reconcile(source);
        presenter.set_selection(
            source,
            selection.range_for(source),
            selection.is_finalized(),
        );
    }

    fn write_selection_clipboard_stdout(
        selection: &SelectionController,
        viewport: &ViewportController,
        surface: &AttachmentSurface,
        presenter: &mut DesktopPresenter,
    ) -> Result<bool, CliError> {
        let Some(source) = viewport.selection_source_identity(surface) else {
            return Ok(false);
        };
        let Some(rows) = viewport.selection_rows(surface) else {
            return Ok(false);
        };
        let Some(write) = selection.extract(source, rows).map_err(selection_error)? else {
            return Ok(false);
        };
        let stdout = io::stdout();
        let mut output = stdout.lock();
        presenter.write_clipboard(&mut output, &write)?;
        Ok(true)
    }

    fn invalidate_selection_stdout(
        selection: &mut SelectionController,
        viewport: &mut ViewportController,
        surface: &AttachmentSurface,
        presenter: &mut DesktopPresenter,
        status: &StatusRenderer,
        transport_state: TerminalViewTransportState,
        pacer: &mut ViewportPresentationPacer,
    ) -> Result<bool, CliError> {
        let source = viewport.selection_source_identity(surface);
        let had_overlay = selection.range_for(source).is_some();
        let previous_flags = presenter.presented_keyboard_flags();
        selection.cancel();
        reconcile_presenter_selection(selection, viewport, surface, presenter);
        let flags_changed =
            previous_flags != presenter.outer_keyboard_flags(surface.modes().keyboard_flags);
        if !had_overlay && !flags_changed {
            return Ok(false);
        }
        pacer.cancel();
        present_surface_stdout(surface, presenter, viewport, status, transport_state)?;
        viewport.observe_presentation();
        pacer.mark_presented(Instant::now());
        Ok(true)
    }

    fn present_surface_with_writer(
        writer: &mut impl Write,
        surface: &AttachmentSurface,
        presenter: &mut DesktopPresenter,
        viewport: &ViewportController,
        status: &StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<bool, CliError> {
        let desired = ComposedFrame::compose(
            &surface.surface,
            presenter.baseline.as_ref(),
            viewport,
            status,
            transport_state,
        )?;
        presenter.present(
            writer,
            desired,
            viewport.next_frame_selection_source_identity(surface),
        )
    }

    fn present_surface_stdout(
        surface: &AttachmentSurface,
        presenter: &mut DesktopPresenter,
        viewport: &ViewportController,
        status: &StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<bool, CliError> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        present_surface_with_writer(
            &mut output,
            surface,
            presenter,
            viewport,
            status,
            transport_state,
        )
    }

    fn install_snapshot_stdout(
        surface: &mut AttachmentSurface,
        presenter: &mut DesktopPresenter,
        snapshot: &TerminalSurfaceSnapshot,
        viewport: &ViewportController,
        status: &StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<(), CliError> {
        let candidate = AttachmentSurface::from_snapshot(snapshot)?;
        let stdout = io::stdout();
        let mut output = stdout.lock();
        present_surface_with_writer(
            &mut output,
            &candidate,
            presenter,
            viewport,
            status,
            transport_state,
        )?;
        *surface = candidate;
        Ok(())
    }

    fn apply_delta_stdout(
        surface: &mut AttachmentSurface,
        presenter: &mut DesktopPresenter,
        delta: &TerminalSurfaceDelta,
        viewport: &mut ViewportController,
        selection: &mut SelectionController,
        status: &StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<DeltaRender, CliError> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        apply_delta_with_writer(
            &mut output,
            surface,
            presenter,
            delta,
            viewport,
            selection,
            status,
            transport_state,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_delta_with_writer(
        writer: &mut impl Write,
        surface: &mut AttachmentSurface,
        presenter: &mut DesktopPresenter,
        delta: &TerminalSurfaceDelta,
        viewport: &mut ViewportController,
        selection: &mut SelectionController,
        status: &StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<DeltaRender, CliError> {
        let Some(candidate) = surface.candidate_after_delta(delta)? else {
            return Ok(DeltaRender::Gap);
        };
        let render_live = viewport.is_live();
        let live_layout = render_live.then(|| {
            ChromeLayout::new(
                status.physical_size,
                status.is_remote(),
                candidate.active_screen(),
            )
        });
        let viewport_plan = viewport.preview_delta(&candidate, delta.scroll_metrics, live_layout);
        let mut candidate_selection = *selection;
        if render_live {
            candidate_selection.cancel();
        }
        candidate_selection.reconcile(viewport_plan.selection_source);
        let selection_presentation =
            candidate_selection.presentation(viewport_plan.selection_source);

        if render_live {
            let desired = ComposedFrame::compose_live_candidate(
                &candidate.surface,
                presenter.baseline.as_ref(),
                viewport,
                LiveViewportProjection::new(
                    viewport_plan.content_size,
                    viewport_plan.gutter_column,
                    viewport_plan
                        .live_metrics
                        .filter(|metrics| metrics.is_valid()),
                ),
                status,
                transport_state,
            )?;
            presenter.present_candidate(
                writer,
                desired,
                viewport_plan.selection_source,
                selection_presentation,
            )?;
        } else {
            presenter.sync_input_modes(writer, candidate.modes(), selection_presentation)?;
        }

        viewport.commit_delta(viewport_plan);
        *selection = candidate_selection;
        *surface = candidate;
        Ok(DeltaRender::Applied)
    }

    #[derive(Clone, Copy)]
    struct CachedPresentationRequest {
        now: Instant,
        force: bool,
    }

    fn mark_cached_viewport_dirty(
        viewport: &ViewportController,
        pacer: &mut ViewportPresentationPacer,
        now: Instant,
    ) -> bool {
        if !viewport.has_complete_presentable_view() {
            pacer.cancel();
            return false;
        }
        pacer.mark_dirty(now);
        true
    }

    fn cancel_unpresentable_cached_viewport(
        viewport: &ViewportController,
        pacer: &mut ViewportPresentationPacer,
    ) {
        if !viewport.has_complete_presentable_view() {
            pacer.cancel();
        }
    }

    fn present_cached_viewport_stdout(
        surface: &AttachmentSurface,
        presenter: &mut DesktopPresenter,
        viewport: &mut ViewportController,
        status: &StatusRenderer,
        transport_state: TerminalViewTransportState,
        pacer: &mut ViewportPresentationPacer,
        request: CachedPresentationRequest,
    ) -> Result<bool, CliError> {
        if !request.force && !pacer.due(request.now) {
            return Ok(false);
        }
        if !viewport.has_complete_presentable_view() {
            pacer.cancel();
            return Ok(false);
        }
        let stdout = io::stdout();
        let mut output = stdout.lock();
        present_surface_with_writer(
            &mut output,
            surface,
            presenter,
            viewport,
            status,
            transport_state,
        )?;
        viewport.observe_presentation();
        pacer.mark_presented(request.now);
        Ok(true)
    }

    fn present_transport_transition_with_writer(
        writer: &mut impl Write,
        surface: &AttachmentSurface,
        presenter: &mut DesktopPresenter,
        viewport: &ViewportController,
        status: &StatusRenderer,
        transport_state: TerminalViewTransportState,
        resumed_from_snapshot: bool,
    ) -> Result<bool, CliError> {
        let resume_sync_is_visually_unchanged = viewport.is_resume_pending()
            && transport_state == TerminalViewTransportState::Synchronizing;
        if resumed_from_snapshot || resume_sync_is_visually_unchanged {
            return Ok(false);
        }
        present_surface_with_writer(
            writer,
            surface,
            presenter,
            viewport,
            status,
            transport_state,
        )?;
        Ok(true)
    }

    fn present_transport_transition_stdout(
        surface: &AttachmentSurface,
        presenter: &mut DesktopPresenter,
        viewport: &ViewportController,
        status: &StatusRenderer,
        transport_state: TerminalViewTransportState,
        resumed_from_snapshot: bool,
    ) -> Result<bool, CliError> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        present_transport_transition_with_writer(
            &mut output,
            surface,
            presenter,
            viewport,
            status,
            transport_state,
            resumed_from_snapshot,
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_viewport_effect(
        effect: ViewportEffect,
        viewport: &mut ViewportController,
        surface: &AttachmentSurface,
        presenter: &mut DesktopPresenter,
        writer: &zterm_daemon::operations::TerminalViewCommandWriter,
        status: &StatusRenderer,
        transport_state: TerminalViewTransportState,
        pacer: &mut ViewportPresentationPacer,
        defer_cached_presentation: bool,
    ) -> Result<bool, CliError> {
        match effect {
            ViewportEffect::None => {
                cancel_unpresentable_cached_viewport(viewport, pacer);
                Ok(false)
            }
            ViewportEffect::Render => {
                let now = Instant::now();
                if mark_cached_viewport_dirty(viewport, pacer, now) {
                    if !defer_cached_presentation {
                        let _ = present_cached_viewport_stdout(
                            surface,
                            presenter,
                            viewport,
                            status,
                            transport_state,
                            pacer,
                            CachedPresentationRequest { now, force: false },
                        )?;
                    }
                } else {
                    pacer.cancel();
                    present_surface_stdout(surface, presenter, viewport, status, transport_state)?;
                    viewport.observe_presentation();
                    pacer.mark_presented(now);
                }
                Ok(false)
            }
            ViewportEffect::RequestHistoryWindow(query) => {
                cancel_unpresentable_cached_viewport(viewport, pacer);
                writer.request_history_window(query).await?;
                Ok(false)
            }
            ViewportEffect::RenderAndRequestHistoryWindow(query) => {
                writer.request_history_window(query).await?;
                let now = Instant::now();
                if mark_cached_viewport_dirty(viewport, pacer, now) && !defer_cached_presentation {
                    let _ = present_cached_viewport_stdout(
                        surface,
                        presenter,
                        viewport,
                        status,
                        transport_state,
                        pacer,
                        CachedPresentationRequest { now, force: false },
                    )?;
                }
                Ok(false)
            }
            ViewportEffect::Resume => {
                pacer.cancel();
                writer.request_sync(surface.revision()).await?;
                Ok(true)
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::fs::File;
        use std::io::Read;
        use std::os::fd::AsRawFd;
        use std::pin::Pin;
        use std::process::{Command, Stdio};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::mpsc;
        use std::task::Poll;

        use nix::fcntl::{FcntlArg, OFlag, fcntl};
        use nix::pty::openpty;
        use nix::sys::signal::{Signal as NixSignal, kill};
        use nix::sys::termios::tcgetattr;
        #[cfg(target_os = "macos")]
        use nix::sys::termios::{LocalFlags, cfgetispeed, cfgetospeed};
        use nix::unistd::Pid;

        use super::*;

        #[derive(Default)]
        struct ViewportFrameWriter {
            bytes: Vec<u8>,
            frames: Vec<Vec<u8>>,
            writes: usize,
            flushes: usize,
        }

        impl Write for ViewportFrameWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(bytes);
                self.frames.push(bytes.to_vec());
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                self.flushes += 1;
                Ok(())
            }
        }

        #[test]
        fn prefix_parser_detaches_escapes_and_preserves_unknown_sequences() {
            let mut parser = PrefixParser::new(Some(0x1d));
            let now = Instant::now();
            assert_eq!(
                parser.feed(b"a\x1d\x1db\x1dx", now),
                vec![PrefixAction::Input(b"a\x1db\x1dx".to_vec())]
            );
            assert_eq!(parser.feed(b"\x1d.", now), vec![PrefixAction::Detach]);
            assert!(parser.detached());
            assert!(parser.feed(b"ignored", now).is_empty());

            let mut transparent = PrefixParser::new(None);
            assert_eq!(
                transparent.feed(b"\x1d.", now),
                vec![PrefixAction::Input(b"\x1d.".to_vec())]
            );
        }

        #[test]
        fn prefix_deadline_flushes_once_and_state_changes_clear_pending_input() {
            let now = Instant::now();
            let mut parser = PrefixParser::new(Some(0x1d));
            assert!(parser.feed(b"\x1d", now).is_empty());
            assert_eq!(parser.deadline(), Some(now + CONTROL_PREFIX_TIMEOUT));
            assert_eq!(parser.flush_pending(), Some(vec![0x1d]));
            assert_eq!(parser.deadline(), None);
            assert_eq!(parser.flush_pending(), None);

            assert!(parser.feed(b"\x1d", now).is_empty());
            parser.clear_pending();
            assert_eq!(parser.deadline(), None);
            assert_eq!(
                parser.feed(b"x", now),
                vec![PrefixAction::Input(b"x".to_vec())]
            );

            assert!(parser.feed(b"\x1d", now).is_empty());
            assert_eq!(
                parser.feed(b"x", now + CONTROL_PREFIX_TIMEOUT),
                vec![PrefixAction::Input(b"\x1dx".to_vec())]
            );
        }

        #[test]
        fn stdin_eof_flushes_a_lone_prefix_only_for_an_active_view() {
            let now = Instant::now();
            let mut active = PrefixParser::new(Some(0x1d));
            assert!(active.feed(b"\x1d", now).is_empty());
            assert_eq!(
                take_pending_active_input(&mut active, TerminalViewTransportState::Active),
                Some(vec![0x1d])
            );
            assert_eq!(active.deadline(), None);

            let mut reconnecting = PrefixParser::new(Some(0x1d));
            assert!(reconnecting.feed(b"\x1d", now).is_empty());
            assert_eq!(
                take_pending_active_input(
                    &mut reconnecting,
                    TerminalViewTransportState::Reconnecting,
                ),
                None
            );
            assert_eq!(reconnecting.deadline(), None);
        }

        #[tokio::test(flavor = "current_thread")]
        async fn stdin_queue_is_exactly_bounded_and_recovers_after_one_receive() {
            let pty = openpty(None, None).expect("open stdin-capacity PTY");
            let input_epoch = InputEpoch::new();
            let mut stdin_pump = StdinPump::start(&pty.slave, input_epoch.clone())
                .expect("start capacity-controlled stdin pump");

            for index in 0..STDIN_CHANNEL_CAPACITY {
                let byte = b'a' + u8::try_from(index).expect("stdin fixture index fits u8");
                assert!(
                    stdin_pump
                        .sender_for_test
                        .try_send(StdinEvent::Bytes {
                            epoch: input_epoch.current(),
                            bytes: vec![byte],
                        })
                        .is_ok(),
                    "every production stdin queue slot is admitted",
                );
            }
            assert_eq!(
                stdin_pump
                    .receiver
                    .as_ref()
                    .expect("live stdin receiver")
                    .len(),
                STDIN_CHANNEL_CAPACITY,
            );
            let Err(tokio::sync::mpsc::error::TrySendError::Full(backpressured)) =
                stdin_pump.sender_for_test.try_send(StdinEvent::Bytes {
                    epoch: input_epoch.current(),
                    bytes: b"i".to_vec(),
                })
            else {
                panic!("the next stdin item must be backpressured at the exact bound");
            };
            assert!(matches!(
                backpressured,
                StdinEvent::Bytes { bytes, .. } if bytes == b"i"
            ));

            assert!(matches!(
                stdin_pump.recv().await,
                Some(StdinEvent::Bytes { bytes, .. }) if bytes == b"a"
            ));
            assert!(
                stdin_pump
                    .sender_for_test
                    .try_send(StdinEvent::Bytes {
                        epoch: input_epoch.current(),
                        bytes: b"i".to_vec(),
                    })
                    .is_ok(),
                "receiving one item recovers one stdin queue slot",
            );
            assert_eq!(
                stdin_pump
                    .receiver
                    .as_ref()
                    .expect("live recovered stdin receiver")
                    .len(),
                STDIN_CHANNEL_CAPACITY,
            );
            stdin_pump
                .shutdown()
                .expect("shutdown capacity-controlled stdin pump");
        }

        #[tokio::test(flavor = "current_thread")]
        async fn repeated_active_fence_joins_live_reader_and_delivers_only_fresh_input() {
            let pty = openpty(None, None).expect("open input-fence PTY");
            let original = tcgetattr(&pty.slave).expect("input-fence attributes");
            let mut raw = original.clone();
            cfmakeraw(&mut raw);
            tcsetattr(&pty.slave, SetArg::TCSANOW, &raw).expect("input-fence raw mode");

            let input_epoch = InputEpoch::new();
            let mut current_epoch = input_epoch.current();
            let mut prefix = PrefixParser::new(Some(0x1d));
            let (reader_test_seam, controls) = StdinReaderTestSeam::with_gates(4);
            let mut controls = VecDeque::from(controls);
            let mut stdin_pump = Some(
                StdinPump::start_with_test_seam(&pty.slave, input_epoch.clone(), reader_test_seam)
                    .expect("start controlled stdin pump"),
            );

            for cycle in 0..2_u8 {
                let stale = vec![b's', b't', b'a', b'l', b'e', b'0' + cycle];
                rustix::io::write(&pty.master, &stale).expect("queue unsynchronized input");
                let stale_control = controls.pop_front().expect("stale-read gate");
                stale_control
                    .observed
                    .recv_timeout(Duration::from_secs(2))
                    .expect("reader reached the deterministic poll/read seam");

                assert!(prefix.feed(b"\x1d", Instant::now()).is_empty());
                let stale_epoch = current_epoch;
                let old_pump = stdin_pump.take().expect("live stdin pump");
                let cancellation_probe = duplicate_cloexec(
                    &old_pump._cancellation_read_guard,
                    "duplicate stdin-fence cancellation probe",
                )
                .expect("cancellation probe");
                let fence_input = duplicate_cloexec(&pty.slave, "duplicate input-fence PTY")
                    .expect("input-fence PTY duplicate");
                let thread_epoch = input_epoch.clone();
                let (result_sender, result_receiver) = sync_channel(1);
                let mut retained = vec![b'k', b'e', b'y', b'0' + cycle];
                let mut viewport = ViewportController {
                    state: ViewportState::ResumePending {
                        retained_input: retained.clone(),
                        snapshot_applied: true,
                        presented_scroll_metrics: None,
                    },
                    content_size: TerminalSize::new(2, 20),
                    live_metrics: None,
                    gutter_column: None,
                    drag_grab_row: None,
                    drag_last_request: None,
                    drag_deferred_target: None,
                    last_presented_scroll_metrics: None,
                    presented_gutter_column: None,
                    discard_window_response: false,
                    window_cache: ViewportCache::new(),
                };
                if cycle == 0 {
                    let mut codec = HostInputCodec::new();
                    assert!(
                        codec
                            .feed(b"\x1b[20")
                            .expect("split paste prefix remains bounded")
                            .is_empty()
                    );
                    assert!(
                        codec
                            .feed(b"0~paste-head\x1d.")
                            .expect("bounded partial paste")
                            .is_empty()
                    );
                    assert!(
                        should_defer_active_for_paste(
                            TerminalViewTransportState::Active,
                            &viewport,
                            &codec,
                        ),
                        "an Active event after Snapshot must wait for the unread paste tail"
                    );
                    assert!(
                        codec
                            .feed(b"-tail\x1b[201")
                            .expect("split paste suffix remains bounded")
                            .is_empty()
                    );
                    let events = codec
                        .feed(b"~")
                        .expect("the final delimiter completes one paste unit");
                    let [HostInputEvent::Paste(paste)] = events.as_slice() else {
                        panic!("split bracketed paste must produce exactly one Paste event");
                    };
                    assert!(!codec.paste_in_progress());
                    viewport
                        .retain_resume_input(paste)
                        .expect("whole paste fits the resume-input bound");
                    retained.extend_from_slice(paste);
                }
                let fence = std::thread::spawn(move || {
                    let mut pump = old_pump;
                    let mut epoch = stale_epoch;
                    let mut parser = prefix;
                    let result = transition_transport_input_state(
                        &fence_input,
                        &thread_epoch,
                        &mut epoch,
                        &mut pump,
                        &mut parser,
                        TerminalViewTransportState::Synchronizing,
                        TerminalViewTransportState::Active,
                        &mut viewport,
                    );
                    let _ = result_sender.send((result, pump, epoch, parser, viewport));
                });

                let bound = rustix::event::Timespec {
                    tv_sec: 2,
                    tv_nsec: 0,
                };
                let mut cancellation = [PollFd::new(&cancellation_probe, PollFlags::IN)];
                assert_eq!(
                    poll(&mut cancellation, Some(&bound)).expect("observe old-pump cancellation"),
                    1,
                    "the Active fence must wake the old reader before joining it"
                );
                assert!(
                    matches!(
                        result_receiver.try_recv(),
                        Err(std::sync::mpsc::TryRecvError::Empty)
                    ),
                    "retained resume input must not become sendable before the old reader joins"
                );
                stale_control
                    .release
                    .send(())
                    .expect("release the old reader after cancellation is visible");
                let (result, returned_pump, returned_epoch, returned_prefix, viewport) =
                    result_receiver
                        .recv_timeout(Duration::from_secs(2))
                        .expect("Active fence completed within its bound");
                fence.join().expect("Active fence thread");
                assert_eq!(
                    result.expect("replace stdin pump after Active fence"),
                    Some(retained)
                );
                assert!(viewport.is_live());
                stdin_pump = Some(returned_pump);
                current_epoch = returned_epoch;
                prefix = returned_prefix;

                assert!(!input_epoch_is_current(stale_epoch, current_epoch));
                assert!(input_epoch_is_current(input_epoch.current(), current_epoch));
                assert_eq!(prefix.deadline(), None);

                let fresh = vec![b'f', b'r', b'e', b's', b'h', b'0' + cycle];
                rustix::io::write(&pty.master, &fresh).expect("write post-fence input");
                let fresh_control = controls.pop_front().expect("fresh-read gate");
                fresh_control
                    .observed
                    .recv_timeout(Duration::from_secs(2))
                    .expect("replacement reader observed post-fence input");
                fresh_control
                    .release
                    .send(())
                    .expect("release replacement reader");
                let event = tokio::time::timeout(
                    Duration::from_secs(2),
                    stdin_pump.as_mut().expect("replacement pump").recv(),
                )
                .await
                .expect("post-fence delivery completed within its bound")
                .expect("replacement pump remained live");
                assert!(matches!(
                    event,
                    StdinEvent::Bytes { epoch, bytes }
                        if epoch == current_epoch && bytes == fresh
                ));
            }

            stdin_pump
                .as_mut()
                .expect("final replacement pump")
                .shutdown()
                .expect("shutdown final replacement pump");
            tcsetattr(&pty.slave, SetArg::TCSANOW, &original)
                .expect("restore input-fence attributes");
        }
        fn test_row(columns: u16, text: &str, style: TerminalStyle) -> TerminalSurfaceRow {
            let mut cells = vec![TerminalCell::default(); usize::from(columns)];
            if let Some(first) = cells.first_mut() {
                first.contents = text.to_owned();
                first.style = style;
            }
            TerminalSurfaceRow {
                cells,
                wrapped: false,
            }
        }

        fn test_snapshot(
            size: TerminalSize,
            active_screen: ActiveScreen,
            revision: Revision,
        ) -> TerminalSurfaceSnapshot {
            let rows = (0..size.rows)
                .map(|row| test_row(size.columns, &row.to_string(), TerminalStyle::default()))
                .collect();
            let scroll_metrics =
                (active_screen == ActiveScreen::Main).then_some(TerminalScrollMetrics {
                    epoch: Revision::new(1),
                    revision,
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 10,
                    viewport_rows: size.rows,
                });
            TerminalSurfaceSnapshot {
                revision,
                surface: TerminalSurface {
                    size,
                    active_screen,
                    rows,
                    cursor: TerminalCursor {
                        row: 0,
                        column: 0,
                        visible: true,
                        style: TerminalStyle::default(),
                    },
                    modes: TerminalModes::default(),
                    scroll_metrics,
                },
            }
        }

        fn install_test_history_window(viewport: &mut ViewportController) {
            let ViewportEffect::RequestHistoryWindow(query) = viewport.navigate(true, 1) else {
                panic!("test history navigation must request a complete window");
            };
            let shape = query
                .response_shape(query.anchor)
                .expect("valid test query");
            let rows = (0..shape.row_count)
                .map(|index| {
                    test_row(
                        query.anchor.viewport.columns,
                        &format!("history-{index}"),
                        TerminalStyle::default(),
                    )
                })
                .collect();
            let effect = viewport
                .apply_view_history_window(TerminalSurfaceHistoryWindowResult::Frame(
                    TerminalSurfaceHistoryWindowFrame {
                        disposition: shape.disposition,
                        anchor: query.anchor,
                        target_offset_from_bottom: shape.target_offset_from_bottom,
                        first_row_from_live_top: shape.first_row_from_live_top,
                        rows,
                    },
                ))
                .expect("install complete test history window");
            assert!(matches!(
                effect,
                ViewportEffect::Render | ViewportEffect::RenderAndRequestHistoryWindow(_)
            ));
        }

        fn finalize_test_selection(
            surface: &AttachmentSurface,
            viewport: &mut ViewportController,
            presenter: &mut DesktopPresenter,
            status: &StatusRenderer,
            output: &mut impl Write,
        ) -> SelectionController {
            let source = viewport
                .selection_source_identity(surface)
                .expect("test viewport has a presented selection source");
            let rows = viewport
                .selection_rows(surface)
                .expect("test viewport has presented semantic rows");
            let mut selection = SelectionController::default();
            selection
                .begin(source, TerminalTextPoint::new(0, 0), rows)
                .expect("begin test selection");
            selection
                .update(source, TerminalTextPoint::new(0, 1), rows)
                .expect("extend test selection");
            selection.finish();
            reconcile_presenter_selection(&mut selection, viewport, surface, presenter);
            present_surface_with_writer(
                output,
                surface,
                presenter,
                viewport,
                status,
                TerminalViewTransportState::Active,
            )
            .expect("present test selection");
            viewport.observe_presentation();
            assert!(selection.is_finalized());
            assert_eq!(presenter.presented_keyboard_flags().bits(), 7);
            selection
        }

        #[test]
        fn resize_coalescer_retains_only_the_latest_non_active_viewport() {
            let initial = TerminalSize::new(24, 80);
            let latest = TerminalSize::new(40, 120);
            let mut coalescer = ResizeCoalescer::new(initial);
            assert_eq!(
                coalescer.observe(
                    TerminalSize::new(20, 60),
                    TerminalViewTransportState::Synchronizing,
                ),
                None
            );
            assert_eq!(
                coalescer.observe(latest, TerminalViewTransportState::Reconnecting),
                None
            );
            assert_eq!(
                coalescer.enter_transport_state(TerminalViewTransportState::Active),
                (TerminalViewTransportState::Synchronizing, Some(latest))
            );
            assert_eq!(
                coalescer.enter_transport_state(TerminalViewTransportState::Active),
                (TerminalViewTransportState::Active, None)
            );
        }

        #[test]
        fn attachment_surface_validates_snapshots_and_applies_deltas_transactionally() {
            let size = TerminalSize::new(2, 4);
            let snapshot = test_snapshot(size, ActiveScreen::Main, Revision::new(4));
            let retained = AttachmentSurface::from_snapshot(&snapshot).expect("valid snapshot");
            let styled = TerminalStyle {
                foreground: TerminalColor::Rgb(7, 8, 9),
                bold: true,
                ..TerminalStyle::default()
            };
            let delta = TerminalSurfaceDelta {
                from_revision: Revision::new(4),
                to_revision: Revision::new(5),
                size,
                active_screen: ActiveScreen::Main,
                row_patches: vec![TerminalSurfaceRowPatch {
                    row: 1,
                    replacement: test_row(size.columns, "changed", styled),
                }],
                cursor: TerminalCursor {
                    row: 1,
                    column: 3,
                    visible: true,
                    style: styled,
                },
                modes: TerminalModes {
                    bracketed_paste: true,
                    ..TerminalModes::default()
                },
                scroll_metrics: Some(TerminalScrollMetrics {
                    epoch: Revision::new(1),
                    revision: Revision::new(5),
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 10,
                    viewport_rows: size.rows,
                }),
            };
            let candidate = retained
                .candidate_after_delta(&delta)
                .expect("valid delta")
                .expect("contiguous delta");
            assert_eq!(retained.revision(), Revision::new(4));
            assert_eq!(candidate.revision(), Revision::new(5));
            assert_eq!(candidate.surface.rows[1].cells[0].contents, "changed");

            let gap = TerminalSurfaceDelta {
                from_revision: Revision::new(3),
                ..delta.clone()
            };
            assert_eq!(
                retained
                    .candidate_after_delta(&gap)
                    .expect("gap is not malformed"),
                None
            );

            let malformed = TerminalSurfaceDelta {
                row_patches: vec![TerminalSurfaceRowPatch {
                    row: 1,
                    replacement: TerminalSurfaceRow {
                        cells: vec![TerminalCell::default(); 3],
                        wrapped: false,
                    },
                }],
                ..delta
            };
            assert!(
                retained.candidate_after_delta(&malformed).is_err(),
                "a malformed patch must not promote a partial surface"
            );
            assert_eq!(
                retained,
                AttachmentSurface::from_snapshot(&snapshot).expect("original snapshot is valid")
            );
        }

        #[test]
        fn semantic_history_uses_one_cache_and_moves_one_line_locally() {
            let physical = TerminalSize::new(4, 10);
            let layout = ChromeLayout::new(physical, false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(2),
                revision: Revision::new(7),
                offset_from_bottom: 0,
                max_offset_from_bottom: 10,
                viewport_rows: layout.child.rows,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            let ViewportEffect::RequestHistoryWindow(query) = viewport.navigate(true, 1) else {
                panic!("the first history miss must issue one semantic window request");
            };
            let shape = query.response_shape(query.anchor).expect("valid query");
            let rows = (0..shape.row_count)
                .map(|index| {
                    test_row(
                        query.anchor.viewport.columns,
                        &index.to_string(),
                        TerminalStyle::default(),
                    )
                })
                .collect();
            let effect = viewport
                .apply_view_history_window(TerminalSurfaceHistoryWindowResult::Frame(
                    TerminalSurfaceHistoryWindowFrame {
                        disposition: shape.disposition,
                        anchor: query.anchor,
                        target_offset_from_bottom: shape.target_offset_from_bottom,
                        first_row_from_live_top: shape.first_row_from_live_top,
                        rows,
                    },
                ))
                .expect("install semantic window");
            assert!(matches!(
                effect,
                ViewportEffect::Render | ViewportEffect::RenderAndRequestHistoryWindow(_)
            ));
            assert_eq!(
                viewport
                    .visible_semantic_history_rows()
                    .expect("complete cached viewport")
                    .0
                    .len(),
                usize::from(layout.child.rows)
            );
            assert_eq!(viewport.window_cache.desired_offset_from_bottom(), 1);

            let effect = viewport.navigate(true, 1);
            assert!(matches!(
                effect,
                ViewportEffect::Render
                    | ViewportEffect::RenderAndRequestHistoryWindow(_)
                    | ViewportEffect::RequestHistoryWindow(_)
            ));
            assert_eq!(viewport.window_cache.desired_offset_from_bottom(), 2);
            assert!(matches!(
                viewport.navigate(false, 2),
                ViewportEffect::Resume
            ));
        }

        #[test]
        fn composed_frame_owns_live_gutter_status_and_alternate_layout() {
            let physical = TerminalSize::new(4, 12);
            let main_layout = ChromeLayout::new(physical, true, ActiveScreen::Main);
            let mut viewport = ViewportController::with_layout(
                main_layout,
                Some(TerminalScrollMetrics {
                    epoch: Revision::new(1),
                    revision: Revision::new(3),
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 4,
                    viewport_rows: main_layout.child.rows,
                }),
            );
            let mut snapshot =
                test_snapshot(main_layout.child, ActiveScreen::Main, Revision::new(3));
            let rightmost = usize::from(main_layout.child.columns - 1);
            snapshot.surface.rows[0].cells[rightmost] = TerminalCell {
                contents: "x".to_owned(),
                style: TerminalStyle {
                    foreground: TerminalColor::Indexed(2),
                    ..TerminalStyle::default()
                },
                ..TerminalCell::default()
            };
            let surface =
                AttachmentSurface::from_snapshot(&snapshot).expect("test snapshot is valid");
            let mut status = StatusRenderer::new(Some("node".to_owned()), physical);
            status.path = TerminalViewConnectionPath::Direct;
            status.rtt_ms = Some(8);
            let frame = ComposedFrame::compose(
                &surface.surface,
                None,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("compose main frame");
            assert_eq!(frame.layout.gutter_column, Some(physical.columns));
            assert_eq!(frame.layout.status_row, Some(physical.rows - 1));
            assert_eq!(
                frame.rows[&0][rightmost],
                snapshot.surface.rows[0].cells[rightmost]
            );
            let status_cells = &frame.rows[&(physical.rows - 1)];
            assert!(status_cells.iter().all(|cell| cell.style.inverse));
            assert_eq!(
                status_cells
                    .iter()
                    .map(|cell| cell.contents.as_str())
                    .collect::<String>(),
                "node | direc"
            );

            let alternate_layout = ChromeLayout::new(physical, true, ActiveScreen::Alternate);
            viewport.set_layout(alternate_layout);
            let alternate = test_snapshot(
                alternate_layout.child,
                ActiveScreen::Alternate,
                Revision::new(4),
            );
            let alternate_frame = ComposedFrame::compose(
                &alternate.surface,
                Some(&frame),
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("compose alternate frame");
            assert_eq!(alternate_frame.layout.gutter_column, None);
            assert_eq!(
                alternate_frame.layout.content_size.columns,
                physical.columns
            );
        }

        #[test]
        fn composed_frame_preserves_wide_spans_combining_text_and_styled_blanks() {
            let size = TerminalSize::new(1, 5);
            let layout = ChromeLayout::new(size, false, ActiveScreen::Alternate);
            let viewport = ViewportController::with_layout(layout, None);
            let style = TerminalStyle {
                background: TerminalColor::Rgb(1, 2, 3),
                underline: true,
                ..TerminalStyle::default()
            };
            let mut snapshot = test_snapshot(size, ActiveScreen::Alternate, Revision::new(2));
            snapshot.surface.rows[0].cells = vec![
                TerminalCell {
                    contents: "e\u{301}".to_owned(),
                    style,
                    ..TerminalCell::default()
                },
                TerminalCell {
                    contents: "界".to_owned(),
                    wide: true,
                    style,
                    ..TerminalCell::default()
                },
                TerminalCell {
                    wide_continuation: true,
                    style,
                    ..TerminalCell::default()
                },
                TerminalCell::default(),
                TerminalCell {
                    style,
                    ..TerminalCell::default()
                },
            ];
            snapshot.validate().expect("valid exact semantic row");
            let status = StatusRenderer::new(None, size);
            let frame = ComposedFrame::compose(
                &snapshot.surface,
                None,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("compose exact row");
            assert_eq!(frame.rows[&0], snapshot.surface.rows[0].cells);
            let blank = vec![TerminalCell::default(); 5];
            assert_eq!(
                semantic_dirty_runs(&snapshot.surface.rows[0].cells, &blank),
                vec![(0, 3), (4, 5)]
            );
        }

        #[test]
        fn compositor_is_sparse_for_huge_physical_row_numbers() {
            let physical = TerminalSize::new(u16::MAX, 6);
            let layout = ChromeLayout::new(physical, true, ActiveScreen::Main);
            let snapshot = test_snapshot(layout.child, ActiveScreen::Main, Revision::new(2));
            let viewport = ViewportController::with_layout(layout, snapshot.surface.scroll_metrics);
            let status = StatusRenderer::new(Some("node".to_owned()), physical);
            let frame = ComposedFrame::compose(
                &snapshot.surface,
                None,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("bounded sparse frame");
            assert_eq!(
                frame.rows.len(),
                usize::from(layout.child.rows).saturating_add(1)
            );
            assert!(frame.rows.contains_key(&(u16::MAX - 1)));
        }

        #[test]
        fn desktop_presenter_is_the_single_atomic_commit_boundary() {
            let size = TerminalSize::new(2, 4);
            let snapshot = test_snapshot(size, ActiveScreen::Alternate, Revision::new(2));
            let surface =
                AttachmentSurface::from_snapshot(&snapshot).expect("test snapshot is valid");
            let viewport = ViewportController::with_layout(
                ChromeLayout::new(size, false, ActiveScreen::Alternate),
                None,
            );
            let status = StatusRenderer::new(None, size);
            let desired = ComposedFrame::compose(
                &surface.surface,
                None,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("test frame composes");
            let mut presenter = DesktopPresenter::default();
            let mut output = ViewportFrameWriter::default();
            assert!(
                presenter
                    .present(&mut output, desired.clone(), None)
                    .expect("initial frame presents")
            );
            assert_eq!((output.writes, output.flushes), (1, 1));
            assert!(output.bytes.starts_with(HOST_SYNC_BEGIN));
            assert!(output.bytes.ends_with(HOST_SYNC_END));
            assert!(find_bytes(&output.bytes, HOST_INPUT_CAPTURE).is_some());
            assert!(
                !presenter
                    .present(&mut output, desired, None)
                    .expect("identical frame is a no-op")
            );
            assert_eq!((output.writes, output.flushes), (1, 1));
        }

        #[test]
        fn presenter_clipboard_write_is_canonical_and_does_not_advance_the_frame() {
            let mut presenter = DesktopPresenter::default();
            let size = TerminalSize::new(1, 2);
            let snapshot = test_snapshot(size, ActiveScreen::Alternate, Revision::new(2));
            let viewport = ViewportController::with_layout(
                ChromeLayout::new(size, false, ActiveScreen::Alternate),
                None,
            );
            let status = StatusRenderer::new(None, size);
            let frame = ComposedFrame::compose(
                &snapshot.surface,
                None,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("compose clipboard baseline frame");
            let mut visual = ViewportFrameWriter::default();
            presenter
                .present(&mut visual, frame, None)
                .expect("present clipboard baseline frame");
            let baseline = presenter.baseline.clone();

            let write = zterm_core::terminal::TerminalClipboardWrite::new("hello".to_owned())
                .expect("clipboard value");
            let mut clipboard = ViewportFrameWriter::default();
            presenter
                .write_clipboard(&mut clipboard, &write)
                .expect("write clipboard escape");
            assert_eq!(clipboard.bytes, b"\x1b]52;c;aGVsbG8=\x07");
            assert_eq!((clipboard.writes, clipboard.flushes), (1, 1));
            assert_eq!(presenter.baseline, baseline);
        }

        #[test]
        fn presenter_clipboard_failure_preserves_the_visual_baseline() {
            struct ClipboardWriteFailure;

            impl Write for ClipboardWriteFailure {
                fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
                    Err(io::Error::other("injected clipboard failure"))
                }

                fn flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }

            let size = TerminalSize::new(1, 2);
            let snapshot = test_snapshot(size, ActiveScreen::Alternate, Revision::new(2));
            let viewport = ViewportController::with_layout(
                ChromeLayout::new(size, false, ActiveScreen::Alternate),
                None,
            );
            let status = StatusRenderer::new(None, size);
            let frame = ComposedFrame::compose(
                &snapshot.surface,
                None,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("compose clipboard failure baseline frame");
            let mut presenter = DesktopPresenter::default();
            let mut output = ViewportFrameWriter::default();
            presenter
                .present(&mut output, frame, None)
                .expect("present clipboard failure baseline frame");
            let baseline = presenter.baseline.clone();
            let write = zterm_core::terminal::TerminalClipboardWrite::new("secret".to_owned())
                .expect("clipboard value");

            let error = presenter
                .write_clipboard(&mut ClipboardWriteFailure, &write)
                .expect_err("injected clipboard write fails");
            assert!(error.to_string().contains("injected clipboard failure"));
            assert_eq!(presenter.baseline, baseline);
            assert!(!error.to_string().contains(write.as_str()));
        }

        #[test]
        fn presenter_selection_overlay_and_keyboard_mode_share_one_frame_commit() {
            let size = TerminalSize::new(1, 4);
            let snapshot = test_snapshot(size, ActiveScreen::Alternate, Revision::new(2));
            let viewport = ViewportController::with_layout(
                ChromeLayout::new(size, false, ActiveScreen::Alternate),
                None,
            );
            let status = StatusRenderer::new(None, size);
            let frame = ComposedFrame::compose(
                &snapshot.surface,
                None,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("compose selection overlay frame");
            let mut presenter = DesktopPresenter::default();
            let source = SelectionSourceIdentity::Live {
                revision: snapshot.revision,
                screen: ActiveScreen::Alternate,
                viewport: size,
            };
            presenter.set_selection(
                Some(source),
                Some(zterm_core::terminal_selection::TerminalTextRange::new(
                    TerminalTextPoint::new(0, 0),
                    TerminalTextPoint::new(0, 1),
                )),
                true,
            );
            let mut output = ViewportFrameWriter::default();
            presenter
                .present(&mut output, frame, Some(source))
                .expect("present selection overlay frame");

            let committed = presenter.baseline.as_ref().expect("committed frame");
            assert!(committed.rows[&0][0].style.inverse);
            assert!(committed.rows[&0][1].style.inverse);
            assert!(!committed.rows[&0][2].style.inverse);
            assert_eq!(committed.modes.keyboard_flags.bits(), 7);
            assert!(find_bytes(&output.bytes, b"\x1b[=7u").is_some());

            presenter.set_selection(None, None, false);
            assert_eq!(
                presenter
                    .outer_keyboard_flags(zterm_core::terminal::TerminalKeyboardFlags::default())
                    .bits(),
                0
            );
            assert_eq!(presenter.presented_keyboard_flags().bits(), 7);
            let frame = ComposedFrame::compose(
                &snapshot.surface,
                presenter.baseline.as_ref(),
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("compose selection removal frame");
            presenter
                .present(&mut output, frame, Some(source))
                .expect("present selection removal frame");
            assert_eq!(presenter.presented_keyboard_flags().bits(), 0);
        }

        #[test]
        fn hidden_history_delta_synchronizes_only_host_input_modes_and_preserves_selection_elevation()
         {
            let physical = TerminalSize::new(3, 8);
            let layout = ChromeLayout::new(physical, false, ActiveScreen::Main);
            let snapshot = test_snapshot(layout.child, ActiveScreen::Main, Revision::new(2));
            let mut surface =
                AttachmentSurface::from_snapshot(&snapshot).expect("valid history surface");
            let mut viewport =
                ViewportController::with_layout(layout, snapshot.surface.scroll_metrics);
            let status = StatusRenderer::new(None, physical);
            let mut presenter = DesktopPresenter::default();
            let mut selection = SelectionController::default();
            let mut output = ViewportFrameWriter::default();

            present_surface_with_writer(
                &mut output,
                &surface,
                &mut presenter,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("present initial live frame");
            viewport.observe_presentation();
            install_test_history_window(&mut viewport);
            present_surface_with_writer(
                &mut output,
                &surface,
                &mut presenter,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("present pinned history frame");
            viewport.observe_presentation();

            let source = viewport
                .selection_source_identity(&surface)
                .expect("presented history has a stable source");
            let rows = viewport
                .selection_rows(&surface)
                .expect("presented history has semantic rows");
            selection
                .begin(source, TerminalTextPoint::new(0, 0), rows)
                .expect("begin history selection");
            selection
                .update(source, TerminalTextPoint::new(0, 1), rows)
                .expect("extend history selection");
            selection.finish();
            reconcile_presenter_selection(&mut selection, &viewport, &surface, &mut presenter);
            present_surface_with_writer(
                &mut output,
                &surface,
                &mut presenter,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("commit selection overlay and local keyboard elevation");
            viewport.observe_presentation();
            assert_eq!(presenter.presented_keyboard_flags().bits(), 7);
            let selected_rows = presenter
                .baseline
                .as_ref()
                .expect("selection frame is committed")
                .rows
                .clone();

            output = ViewportFrameWriter::default();
            let child_flags = zterm_core::terminal::TerminalKeyboardFlags::from_bits(9)
                .expect("valid child keyboard flags");
            let delta = TerminalSurfaceDelta {
                from_revision: Revision::new(2),
                to_revision: Revision::new(3),
                size: layout.child,
                active_screen: ActiveScreen::Main,
                row_patches: vec![TerminalSurfaceRowPatch {
                    row: 0,
                    replacement: test_row(
                        layout.child.columns,
                        "hidden-live-change",
                        TerminalStyle::default(),
                    ),
                }],
                cursor: snapshot.surface.cursor,
                modes: TerminalModes {
                    application_cursor: true,
                    application_keypad: true,
                    bracketed_paste: true,
                    focus_reporting: true,
                    keyboard_flags: child_flags,
                    ..TerminalModes::default()
                },
                scroll_metrics: Some(TerminalScrollMetrics {
                    epoch: Revision::new(1),
                    revision: Revision::new(3),
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 10,
                    viewport_rows: layout.child.rows,
                }),
            };
            assert_eq!(
                apply_delta_with_writer(
                    &mut output,
                    &mut surface,
                    &mut presenter,
                    &delta,
                    &mut viewport,
                    &mut selection,
                    &status,
                    TerminalViewTransportState::Active,
                )
                .expect("apply hidden history delta"),
                DeltaRender::Applied
            );
            assert_eq!(output.bytes, b"\x1b[?1h\x1b=\x1b[?2004h\x1b[?1004h\x1b[=9u");
            assert_eq!((output.writes, output.flushes), (1, 1));
            assert_eq!(surface.revision(), Revision::new(3));
            assert!(surface.modes().application_cursor);
            assert!(surface.modes().application_keypad);
            assert!(surface.modes().bracketed_paste);
            assert!(surface.modes().focus_reporting);
            assert_eq!(surface.modes().keyboard_flags, child_flags);
            assert_eq!(presenter.presented_keyboard_flags(), child_flags);
            let committed = presenter
                .baseline
                .as_ref()
                .expect("mode commit retains frame");
            assert_eq!(committed.rows, selected_rows);
            assert!(committed.modes.application_cursor);
            assert!(committed.modes.application_keypad);
            assert!(committed.modes.bracketed_paste);
            assert!(committed.modes.focus_reporting);
            assert_eq!(committed.modes.keyboard_flags, child_flags);
            assert!(find_bytes(&output.bytes, HOST_SYNC_BEGIN).is_none());
            assert!(
                !present_surface_with_writer(
                    &mut output,
                    &surface,
                    &mut presenter,
                    &viewport,
                    &status,
                    TerminalViewTransportState::Active,
                )
                .expect("accurate mode baseline makes the next full presentation a no-op")
            );
            assert_eq!((output.writes, output.flushes), (1, 1));

            output = ViewportFrameWriter::default();
            let delta = TerminalSurfaceDelta {
                from_revision: Revision::new(3),
                to_revision: Revision::new(4),
                size: layout.child,
                active_screen: ActiveScreen::Main,
                row_patches: Vec::new(),
                cursor: snapshot.surface.cursor,
                modes: TerminalModes::default(),
                scroll_metrics: Some(TerminalScrollMetrics {
                    epoch: Revision::new(1),
                    revision: Revision::new(4),
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 10,
                    viewport_rows: layout.child.rows,
                }),
            };
            assert_eq!(
                apply_delta_with_writer(
                    &mut output,
                    &mut surface,
                    &mut presenter,
                    &delta,
                    &mut viewport,
                    &mut selection,
                    &status,
                    TerminalViewTransportState::Active,
                )
                .expect("restore child legacy mode behind history"),
                DeltaRender::Applied
            );
            assert_eq!(output.bytes, b"\x1b[?1l\x1b>\x1b[?2004l\x1b[?1004l\x1b[=7u");
            assert_eq!((output.writes, output.flushes), (1, 1));
            assert_eq!(surface.modes().keyboard_flags.bits(), 0);
            assert_eq!(presenter.presented_keyboard_flags().bits(), 7);
            assert_eq!(
                presenter
                    .baseline
                    .as_ref()
                    .expect("selection frame remains committed")
                    .rows,
                selected_rows
            );

            output = ViewportFrameWriter::default();
            let routed_only = TerminalModes {
                alternate_scroll: true,
                mouse_mode: TerminalMouseMode::PressRelease,
                mouse_encoding: TerminalMouseEncoding::Sgr,
                ..TerminalModes::default()
            };
            let delta = TerminalSurfaceDelta {
                from_revision: Revision::new(4),
                to_revision: Revision::new(5),
                size: layout.child,
                active_screen: ActiveScreen::Main,
                row_patches: vec![TerminalSurfaceRowPatch {
                    row: 0,
                    replacement: test_row(
                        layout.child.columns,
                        "routed-live",
                        TerminalStyle::default(),
                    ),
                }],
                cursor: snapshot.surface.cursor,
                modes: routed_only,
                scroll_metrics: Some(TerminalScrollMetrics {
                    epoch: Revision::new(1),
                    revision: Revision::new(5),
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 10,
                    viewport_rows: layout.child.rows,
                }),
            };
            assert_eq!(
                apply_delta_with_writer(
                    &mut output,
                    &mut surface,
                    &mut presenter,
                    &delta,
                    &mut viewport,
                    &mut selection,
                    &status,
                    TerminalViewTransportState::Active,
                )
                .expect("install routed-only child modes behind history"),
                DeltaRender::Applied
            );
            assert_eq!((output.writes, output.flushes), (0, 0));
            assert!(output.bytes.is_empty());
            assert_eq!(surface.modes(), routed_only);
            let committed = presenter
                .baseline
                .as_ref()
                .expect("routed modes do not alter physical baseline");
            assert_eq!(committed.rows, selected_rows);
            assert!(!committed.modes.alternate_scroll);
            assert_eq!(committed.modes.mouse_mode, TerminalMouseMode::None);
            assert_eq!(
                committed.modes.mouse_encoding,
                TerminalMouseEncoding::default()
            );
            assert!(
                !present_surface_with_writer(
                    &mut output,
                    &surface,
                    &mut presenter,
                    &viewport,
                    &status,
                    TerminalViewTransportState::Active,
                )
                .expect("routed-only semantic modes never trigger a physical repaint")
            );
            assert_eq!((output.writes, output.flushes), (0, 0));
        }

        #[test]
        fn hidden_incompatible_anchor_retires_selection_in_the_sole_mode_transaction() {
            for (case, epoch, maximum) in [
                ("epoch change", Revision::new(2), 10),
                ("history shrink", Revision::new(1), 0),
            ] {
                let physical = TerminalSize::new(3, 8);
                let layout = ChromeLayout::new(physical, false, ActiveScreen::Main);
                let snapshot = test_snapshot(layout.child, ActiveScreen::Main, Revision::new(2));
                let mut surface =
                    AttachmentSurface::from_snapshot(&snapshot).expect("valid history surface");
                let mut viewport =
                    ViewportController::with_layout(layout, snapshot.surface.scroll_metrics);
                let status = StatusRenderer::new(None, physical);
                let mut presenter = DesktopPresenter::default();
                let mut setup_output = ViewportFrameWriter::default();
                present_surface_with_writer(
                    &mut setup_output,
                    &surface,
                    &mut presenter,
                    &viewport,
                    &status,
                    TerminalViewTransportState::Active,
                )
                .expect("present initial live frame");
                viewport.observe_presentation();
                install_test_history_window(&mut viewport);
                present_surface_with_writer(
                    &mut setup_output,
                    &surface,
                    &mut presenter,
                    &viewport,
                    &status,
                    TerminalViewTransportState::Active,
                )
                .expect("present pinned history frame");
                viewport.observe_presentation();
                let mut selection = finalize_test_selection(
                    &surface,
                    &mut viewport,
                    &mut presenter,
                    &status,
                    &mut setup_output,
                );
                let selected_rows = presenter
                    .baseline
                    .as_ref()
                    .expect("selected history frame is committed")
                    .rows
                    .clone();

                let delta = TerminalSurfaceDelta {
                    from_revision: Revision::new(2),
                    to_revision: Revision::new(3),
                    size: layout.child,
                    active_screen: ActiveScreen::Main,
                    row_patches: vec![TerminalSurfaceRowPatch {
                        row: 0,
                        replacement: test_row(
                            layout.child.columns,
                            "hidden-new",
                            TerminalStyle::default(),
                        ),
                    }],
                    cursor: snapshot.surface.cursor,
                    modes: TerminalModes::default(),
                    scroll_metrics: Some(TerminalScrollMetrics {
                        epoch,
                        revision: Revision::new(3),
                        offset_from_bottom: 0,
                        max_offset_from_bottom: maximum,
                        viewport_rows: layout.child.rows,
                    }),
                };
                let mut output = ViewportFrameWriter::default();
                assert_eq!(
                    apply_delta_with_writer(
                        &mut output,
                        &mut surface,
                        &mut presenter,
                        &delta,
                        &mut viewport,
                        &mut selection,
                        &status,
                        TerminalViewTransportState::Active,
                    )
                    .unwrap_or_else(|error| panic!("{case} delta failed: {error}")),
                    DeltaRender::Applied
                );

                assert_eq!(output.bytes, b"\x1b[=0u", "{case}");
                assert_eq!((output.writes, output.flushes), (1, 1), "{case}");
                assert!(
                    find_bytes(&output.bytes, HOST_SYNC_BEGIN).is_none(),
                    "{case}"
                );
                assert_eq!(surface.revision(), Revision::new(3), "{case}");
                assert!(!selection.is_finalized(), "{case}");
                assert_eq!(viewport.selection_source_identity(&surface), None, "{case}");
                assert!(viewport.window_cache.presented_rows().is_none(), "{case}");
                assert_eq!(presenter.presented_keyboard_flags().bits(), 0, "{case}");
                let committed = presenter
                    .baseline
                    .as_ref()
                    .expect("mode-only transaction retains the visual baseline");
                assert_eq!(committed.rows, selected_rows, "{case}");
                assert_eq!(committed.modes.keyboard_flags.bits(), 0, "{case}");

                let mut no_io = ViewportFrameWriter::default();
                assert!(
                    !presenter
                        .sync_input_modes(
                            &mut no_io,
                            surface.modes(),
                            selection.presentation(viewport.selection_source_identity(&surface)),
                        )
                        .expect("committed candidate needs no second mode sync"),
                    "{case}"
                );
                assert_eq!((no_io.writes, no_io.flushes), (0, 0), "{case}");
            }
        }

        #[test]
        fn hidden_delta_io_failure_keeps_every_semantic_candidate_uncommitted() {
            #[derive(Clone, Copy, Eq, PartialEq)]
            enum FailurePoint {
                Write,
                Flush,
            }

            struct TransactionFailure {
                point: FailurePoint,
                failed: bool,
                bytes: Vec<u8>,
                writes: usize,
                flushes: usize,
            }

            impl Write for TransactionFailure {
                fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    self.writes += 1;
                    if self.point == FailurePoint::Write && !self.failed {
                        self.failed = true;
                        return Err(io::Error::other("injected input mode write failure"));
                    }
                    self.bytes.extend_from_slice(bytes);
                    Ok(bytes.len())
                }

                fn flush(&mut self) -> io::Result<()> {
                    self.flushes += 1;
                    if self.point == FailurePoint::Flush && !self.failed {
                        self.failed = true;
                        return Err(io::Error::other("injected input mode flush failure"));
                    }
                    Ok(())
                }
            }

            for point in [FailurePoint::Write, FailurePoint::Flush] {
                let physical = TerminalSize::new(3, 8);
                let layout = ChromeLayout::new(physical, false, ActiveScreen::Main);
                let snapshot = test_snapshot(layout.child, ActiveScreen::Main, Revision::new(2));
                let mut surface =
                    AttachmentSurface::from_snapshot(&snapshot).expect("valid history surface");
                let mut viewport =
                    ViewportController::with_layout(layout, snapshot.surface.scroll_metrics);
                let status = StatusRenderer::new(None, physical);
                let mut presenter = DesktopPresenter::default();
                let mut setup_output = ViewportFrameWriter::default();
                present_surface_with_writer(
                    &mut setup_output,
                    &surface,
                    &mut presenter,
                    &viewport,
                    &status,
                    TerminalViewTransportState::Active,
                )
                .expect("present initial frame");
                viewport.observe_presentation();
                install_test_history_window(&mut viewport);
                present_surface_with_writer(
                    &mut setup_output,
                    &surface,
                    &mut presenter,
                    &viewport,
                    &status,
                    TerminalViewTransportState::Active,
                )
                .expect("present history frame");
                viewport.observe_presentation();
                let mut selection = finalize_test_selection(
                    &surface,
                    &mut viewport,
                    &mut presenter,
                    &status,
                    &mut setup_output,
                );
                let selection_before = selection;
                let anchor_before = viewport.window_cache.anchor();
                let desired_before = viewport.window_cache.desired_offset_from_bottom();
                let source_before = viewport.selection_source_identity(&surface);
                let metrics_before = viewport.live_metrics;

                let delta = TerminalSurfaceDelta {
                    from_revision: Revision::new(2),
                    to_revision: Revision::new(3),
                    size: layout.child,
                    active_screen: ActiveScreen::Main,
                    row_patches: Vec::new(),
                    cursor: snapshot.surface.cursor,
                    modes: TerminalModes::default(),
                    scroll_metrics: Some(TerminalScrollMetrics {
                        epoch: Revision::new(2),
                        revision: Revision::new(3),
                        offset_from_bottom: 0,
                        max_offset_from_bottom: 4,
                        viewport_rows: layout.child.rows,
                    }),
                };
                let mut failure = TransactionFailure {
                    point,
                    failed: false,
                    bytes: Vec::new(),
                    writes: 0,
                    flushes: 0,
                };
                let error = apply_delta_with_writer(
                    &mut failure,
                    &mut surface,
                    &mut presenter,
                    &delta,
                    &mut viewport,
                    &mut selection,
                    &status,
                    TerminalViewTransportState::Active,
                )
                .expect_err("failed physical mode commit rejects every candidate state");
                assert!(error.to_string().contains(match point {
                    FailurePoint::Write => "input mode write failure",
                    FailurePoint::Flush => "input mode flush failure",
                }));
                assert_eq!(failure.writes, 1);
                match point {
                    FailurePoint::Write => {
                        assert!(failure.bytes.is_empty());
                        assert_eq!(failure.flushes, 0);
                    }
                    FailurePoint::Flush => {
                        assert_eq!(failure.bytes, b"\x1b[=0u");
                        assert_eq!(failure.flushes, 1);
                    }
                }
                assert_eq!(surface.revision(), Revision::new(2));
                assert_eq!(viewport.window_cache.anchor(), anchor_before);
                assert_eq!(
                    viewport.window_cache.desired_offset_from_bottom(),
                    desired_before
                );
                assert_eq!(viewport.selection_source_identity(&surface), source_before);
                assert_eq!(viewport.live_metrics, metrics_before);
                assert_eq!(selection, selection_before);
                assert!(selection.is_finalized());
                assert_eq!(presenter.presented_keyboard_flags().bits(), 7);
                assert!(presenter.baseline.is_none());

                let mut recovery = ViewportFrameWriter::default();
                assert!(
                    present_surface_with_writer(
                        &mut recovery,
                        &surface,
                        &mut presenter,
                        &viewport,
                        &status,
                        TerminalViewTransportState::Active,
                    )
                    .expect("unknown baseline forces a complete pre-delta recovery frame")
                );
                assert!(find_bytes(&recovery.bytes, b"\x1b[2J").is_some());
                assert!(find_bytes(&recovery.bytes, HOST_INPUT_CAPTURE).is_some());
                assert!(find_bytes(&recovery.bytes, b"\x1b[=7u").is_some());
                assert_eq!(presenter.presented_keyboard_flags().bits(), 7);
            }
        }

        #[test]
        fn live_delta_presentation_failure_keeps_surface_viewport_and_selection_uncommitted() {
            struct FlushFailure {
                bytes: Vec<u8>,
            }

            impl Write for FlushFailure {
                fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    self.bytes.extend_from_slice(bytes);
                    Ok(bytes.len())
                }

                fn flush(&mut self) -> io::Result<()> {
                    Err(io::Error::other("injected live delta flush failure"))
                }
            }

            let physical = TerminalSize::new(3, 8);
            let layout = ChromeLayout::new(physical, false, ActiveScreen::Main);
            let snapshot = test_snapshot(layout.child, ActiveScreen::Main, Revision::new(2));
            let mut surface =
                AttachmentSurface::from_snapshot(&snapshot).expect("valid live surface");
            let mut viewport =
                ViewportController::with_layout(layout, snapshot.surface.scroll_metrics);
            let status = StatusRenderer::new(None, physical);
            let mut presenter = DesktopPresenter::default();
            let mut setup_output = ViewportFrameWriter::default();
            present_surface_with_writer(
                &mut setup_output,
                &surface,
                &mut presenter,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("present initial live frame");
            viewport.observe_presentation();
            let mut selection = finalize_test_selection(
                &surface,
                &mut viewport,
                &mut presenter,
                &status,
                &mut setup_output,
            );
            let selection_before = selection;
            let metrics_before = viewport.live_metrics;
            let anchor_before = viewport.window_cache.anchor();
            let source_before = viewport.selection_source_identity(&surface);

            let delta = TerminalSurfaceDelta {
                from_revision: Revision::new(2),
                to_revision: Revision::new(3),
                size: layout.child,
                active_screen: ActiveScreen::Main,
                row_patches: Vec::new(),
                cursor: snapshot.surface.cursor,
                modes: TerminalModes {
                    application_cursor: true,
                    ..TerminalModes::default()
                },
                scroll_metrics: Some(TerminalScrollMetrics {
                    epoch: Revision::new(1),
                    revision: Revision::new(3),
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 11,
                    viewport_rows: layout.child.rows,
                }),
            };
            let mut failure = FlushFailure { bytes: Vec::new() };
            let error = apply_delta_with_writer(
                &mut failure,
                &mut surface,
                &mut presenter,
                &delta,
                &mut viewport,
                &mut selection,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect_err("failed live frame rejects every candidate state");
            assert!(
                error.to_string().contains("live delta flush failure"),
                "unexpected live delta failure: {error}"
            );
            assert_eq!(surface.revision(), Revision::new(2));
            assert_eq!(surface.active_screen(), ActiveScreen::Main);
            assert_eq!(viewport.content_size, layout.child);
            assert_eq!(viewport.gutter_column, layout.gutter_column);
            assert_eq!(viewport.live_metrics, metrics_before);
            assert_eq!(viewport.window_cache.anchor(), anchor_before);
            assert_eq!(viewport.selection_source_identity(&surface), source_before);
            assert_eq!(selection, selection_before);
            assert!(selection.is_finalized());
            assert_eq!(presenter.presented_keyboard_flags().bits(), 7);
            assert!(presenter.baseline.is_none());

            let mut recovery = ViewportFrameWriter::default();
            assert!(
                present_surface_with_writer(
                    &mut recovery,
                    &surface,
                    &mut presenter,
                    &viewport,
                    &status,
                    TerminalViewTransportState::Active,
                )
                .expect("full recovery presents the complete pre-delta state")
            );
            assert!(find_bytes(&recovery.bytes, b"\x1b[2J").is_some());
            assert!(find_bytes(&recovery.bytes, b"\x1b[=7u").is_some());
            assert_eq!(presenter.presented_keyboard_flags().bits(), 7);
        }

        #[test]
        fn presenter_forgets_baseline_after_flush_failure_and_repaints_fully() {
            struct FlushFailOnce {
                bytes: Vec<u8>,
                writes: usize,
                flushes: usize,
                fail_next_flush: bool,
            }

            impl Write for FlushFailOnce {
                fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    self.writes += 1;
                    self.bytes.extend_from_slice(bytes);
                    Ok(bytes.len())
                }

                fn flush(&mut self) -> io::Result<()> {
                    self.flushes += 1;
                    if self.fail_next_flush {
                        self.fail_next_flush = false;
                        Err(io::Error::other("injected flush failure"))
                    } else {
                        Ok(())
                    }
                }
            }

            let size = TerminalSize::new(1, 3);
            let snapshot = test_snapshot(size, ActiveScreen::Alternate, Revision::new(2));
            let viewport = ViewportController::with_layout(
                ChromeLayout::new(size, false, ActiveScreen::Alternate),
                None,
            );
            let status = StatusRenderer::new(None, size);
            let desired = ComposedFrame::compose(
                &snapshot.surface,
                None,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("test frame composes");
            let mut presenter = DesktopPresenter::default();
            let mut writer = FlushFailOnce {
                bytes: Vec::new(),
                writes: 0,
                flushes: 0,
                fail_next_flush: true,
            };
            assert!(
                presenter
                    .present(&mut writer, desired.clone(), None)
                    .is_err()
            );
            assert!(presenter.baseline.is_none());

            writer.bytes.clear();
            assert!(
                presenter
                    .present(&mut writer, desired, None)
                    .expect("retry frame presents")
            );
            assert!(
                find_bytes(&writer.bytes, b"\x1b[2J").is_some(),
                "unknown baseline requires a full clear and repaint"
            );
            assert!(presenter.baseline.is_some());
        }

        #[test]
        fn presenter_forgets_baseline_after_partial_write_failure_and_repaints_fully() {
            struct PartialWriteFailOnce {
                bytes: Vec<u8>,
                writes: usize,
                flushes: usize,
            }

            impl Write for PartialWriteFailOnce {
                fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    self.writes += 1;
                    match self.writes {
                        1 => {
                            let written = bytes.len().min(4);
                            self.bytes.extend_from_slice(&bytes[..written]);
                            Ok(written)
                        }
                        2 => Err(io::Error::other("injected partial write failure")),
                        _ => {
                            self.bytes.extend_from_slice(bytes);
                            Ok(bytes.len())
                        }
                    }
                }

                fn flush(&mut self) -> io::Result<()> {
                    self.flushes += 1;
                    Ok(())
                }
            }

            let size = TerminalSize::new(1, 3);
            let snapshot = test_snapshot(size, ActiveScreen::Alternate, Revision::new(2));
            let viewport = ViewportController::with_layout(
                ChromeLayout::new(size, false, ActiveScreen::Alternate),
                None,
            );
            let status = StatusRenderer::new(None, size);
            let desired = ComposedFrame::compose(
                &snapshot.surface,
                None,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("test frame composes");
            let mut presenter = DesktopPresenter::default();
            let mut writer = PartialWriteFailOnce {
                bytes: Vec::new(),
                writes: 0,
                flushes: 0,
            };
            let error = presenter
                .present(&mut writer, desired.clone(), None)
                .expect_err("the injected partial write fails");
            assert!(error.to_string().contains("injected partial write failure"));
            assert!(presenter.baseline.is_none());
            assert!(writer.bytes.ends_with(HOST_SYNC_END));

            writer.bytes.clear();
            assert!(
                presenter
                    .present(&mut writer, desired, None)
                    .expect("retry frame presents")
            );
            assert!(
                find_bytes(&writer.bytes, b"\x1b[2J").is_some(),
                "unknown baseline requires a full clear and repaint"
            );
            assert!(presenter.baseline.is_some());
        }

        #[test]
        fn host_input_routing_is_application_neutral_and_has_one_mouse_owner() {
            let mut codec = HostInputCodec::new();
            let wheel_raw = b"\x1b[<64;5;4M";
            let events = codec.feed(wheel_raw).expect("SGR wheel input");
            let Some(HostInputEvent::Mouse(wheel)) = events.first() else {
                panic!("wheel must decode as one mouse event");
            };
            let shell_modes = TerminalModes::default();
            assert!(history_owns_gestures(ActiveScreen::Main, shell_modes));
            assert_eq!(
                route_mouse_to_child(wheel, ActiveScreen::Main, shell_modes),
                None
            );

            let child_mouse = TerminalModes {
                mouse_mode: TerminalMouseMode::PressRelease,
                mouse_encoding: TerminalMouseEncoding::Sgr,
                ..TerminalModes::default()
            };
            assert!(!history_owns_gestures(ActiveScreen::Main, child_mouse));
            assert_eq!(
                route_mouse_to_child(wheel, ActiveScreen::Main, child_mouse),
                Some(wheel_raw.to_vec())
            );

            let alternate_scroll = TerminalModes {
                alternate_scroll: true,
                application_cursor: true,
                ..TerminalModes::default()
            };
            assert_eq!(
                route_mouse_to_child(wheel, ActiveScreen::Alternate, alternate_scroll),
                Some(b"\x1bOA".to_vec())
            );
        }

        #[test]
        fn host_codec_bounds_and_preserves_fragmented_csi_u_and_malformed_csi() {
            let mut codec = HostInputCodec::new();
            assert!(
                codec
                    .feed(b"\x1b[99:67;")
                    .expect("bounded CSI-u prefix")
                    .is_empty()
            );
            let events = codec.feed(b"6:2u").expect("complete CSI-u event");
            let [HostInputEvent::EnhancedKey(key)] = events.as_slice() else {
                panic!("fragmented enhanced key must decode once");
            };
            assert_eq!(key.raw, b"\x1b[99:67;6:2u");
            assert_eq!(key.kind, KeyEventKind::Repeat);

            let malformed = codec
                .feed(b"\x1b[99;0u")
                .expect("malformed CSI-u input is preserved");
            assert_eq!(
                malformed,
                vec![HostInputEvent::Bytes(b"\x1b[99;0u".to_vec())]
            );
            let malformed_with_etx = codec
                .feed(b"\x1b[99;\x03u")
                .expect("malformed CSI-u containing ETX is preserved");
            assert_eq!(
                malformed_with_etx,
                vec![HostInputEvent::Bytes(b"\x1b[99;\x03u".to_vec())],
                "an ETX inside a malformed framed sequence must not be guessed as a copy key"
            );
            assert_eq!(
                codec.feed(b"a\x03b").expect("legacy bytes decode"),
                vec![
                    HostInputEvent::Bytes(b"a".to_vec()),
                    HostInputEvent::LegacyCtrlC,
                    HostInputEvent::Bytes(b"b".to_vec()),
                ]
            );

            let oversized = [b"\x1b[".as_slice(), &[b'1'; HOST_SEQUENCE_BOUND]].concat();
            let drained = codec
                .feed(&oversized)
                .expect("oversized host sequence drains safely");
            assert!(!drained.is_empty());
        }

        #[test]
        fn enhanced_keyboard_router_copies_once_and_preserves_forwarding_contracts() {
            let flags_seven = zterm_core::terminal::TerminalKeyboardFlags::from_bits(7)
                .expect("protocol-complete local copy mode");
            let child_legacy = TerminalModes::default();

            let press =
                EnhancedKey::parse(b"\x1b[99:67;6:1u".to_vec()).expect("Ctrl+Shift+C press");
            let repeat =
                EnhancedKey::parse(b"\x1b[99:67;6:2u".to_vec()).expect("Ctrl+Shift+C repeat");
            let release =
                EnhancedKey::parse(b"\x1b[99:67;6:3u".to_vec()).expect("Ctrl+Shift+C release");
            let mut lease = CopyKeyLease::default();
            assert!(matches!(
                route_enhanced_input(&press, child_legacy, flags_seven, true, &mut lease),
                KeyboardRoute::Copy
            ));
            assert!(matches!(
                route_enhanced_input(&repeat, child_legacy, flags_seven, true, &mut lease),
                KeyboardRoute::Consume
            ));
            assert!(matches!(
                route_enhanced_input(&release, child_legacy, flags_seven, true, &mut lease),
                KeyboardRoute::Consume
            ));

            let super_copy = EnhancedKey::parse(b"\x1b[99;9:1u".to_vec()).expect("Super+C press");
            assert!(matches!(
                route_enhanced_input(
                    &super_copy,
                    child_legacy,
                    flags_seven,
                    true,
                    &mut CopyKeyLease::default(),
                ),
                KeyboardRoute::Copy
            ));

            let raw = b"\x1b[120;5:1u";
            let ctrl_x = EnhancedKey::parse(raw.to_vec()).expect("Ctrl+X");
            let child_enhanced = TerminalModes {
                keyboard_flags: flags_seven,
                ..TerminalModes::default()
            };
            let KeyboardRoute::Forward {
                bytes,
                clear_selection,
                reinterpret_legacy,
            } = route_enhanced_input(
                &ctrl_x,
                child_enhanced,
                flags_seven,
                true,
                &mut CopyKeyLease::default(),
            )
            else {
                panic!("matching modes must forward a non-copy key");
            };
            assert_eq!(bytes, raw);
            assert!(clear_selection);
            assert!(!reinterpret_legacy);

            let KeyboardRoute::Forward {
                bytes,
                clear_selection,
                reinterpret_legacy,
            } = route_enhanced_input(
                &ctrl_x,
                child_legacy,
                flags_seven,
                true,
                &mut CopyKeyLease::default(),
            )
            else {
                panic!("temporary local elevation must downgrade a non-copy key");
            };
            assert_eq!(bytes, vec![0x18]);
            assert!(clear_selection);
            assert!(reinterpret_legacy);

            let raw_unknown = b"\x1b[57358;1:2u";
            let unknown = EnhancedKey::parse(raw_unknown.to_vec()).expect("unknown key");
            let KeyboardRoute::Forward {
                bytes,
                reinterpret_legacy,
                ..
            } = route_enhanced_input(
                &unknown,
                child_legacy,
                flags_seven,
                true,
                &mut CopyKeyLease::default(),
            )
            else {
                panic!("unknown input must remain forwardable");
            };
            assert_eq!(bytes, raw_unknown);
            assert!(reinterpret_legacy);
            assert_eq!(
                host_events_from_legacy_bytes(bytes),
                vec![HostInputEvent::Bytes(raw_unknown.to_vec())]
            );

            let child_enhanced = TerminalModes {
                keyboard_flags: zterm_core::terminal::TerminalKeyboardFlags::from_bits(9)
                    .expect("valid child keyboard flags"),
                ..TerminalModes::default()
            };
            let KeyboardRoute::Forward {
                bytes,
                reinterpret_legacy,
                ..
            } = route_enhanced_input(
                &ctrl_x,
                child_enhanced,
                zterm_core::terminal::TerminalKeyboardFlags::default(),
                false,
                &mut CopyKeyLease::default(),
            )
            else {
                panic!("an arbitrary physical/child mismatch remains byte preserving");
            };
            assert_eq!(bytes, raw);
            assert!(!reinterpret_legacy);
        }

        fn sgr_mouse(raw: &[u8]) -> SgrMouse {
            SgrMouse::parse(raw.to_vec()).expect("valid SGR mouse fixture")
        }

        #[test]
        fn pointer_router_selects_exactly_one_owner_for_selection_child_history_and_gutter() {
            let physical = TerminalSize::new(3, 8);
            let layout = ChromeLayout::new(physical, false, ActiveScreen::Main);
            let snapshot = test_snapshot(layout.child, ActiveScreen::Main, Revision::new(7));
            let mut surface =
                AttachmentSurface::from_snapshot(&snapshot).expect("valid pointer surface");
            let mut viewport =
                ViewportController::with_layout(layout, snapshot.surface.scroll_metrics);
            let mut selection = SelectionController::default();

            assert!(matches!(
                route_pointer(
                    &sgr_mouse(b"\x1b[<0;1;1M"),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route selection press"),
                PointerRoute::SelectionChanged
            ));
            assert!(matches!(
                route_pointer(
                    &sgr_mouse(b"\x1b[<32;3;1M"),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route selection motion"),
                PointerRoute::SelectionChanged
            ));
            assert!(matches!(
                route_pointer(
                    &sgr_mouse(b"\x1b[<0;3;1m"),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route selection release"),
                PointerRoute::SelectionChanged
            ));
            assert!(selection.is_finalized());

            assert!(matches!(
                route_pointer(
                    &sgr_mouse(b"\x1b[<64;2;2M"),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route history wheel"),
                PointerRoute::Viewport(_)
            ));
            assert!(!selection.is_finalized());

            surface.surface.modes.mouse_mode = TerminalMouseMode::PressRelease;
            surface.surface.modes.mouse_encoding = TerminalMouseEncoding::Sgr;
            let child_raw = b"\x1b[<0;2;2M";
            assert!(matches!(
                route_pointer(
                    &sgr_mouse(child_raw),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route child press after local selection"),
                PointerRoute::Ignore
            ));

            let mut viewport =
                ViewportController::with_layout(layout, snapshot.surface.scroll_metrics);
            let mut selection = SelectionController::default();
            assert!(matches!(
                route_pointer(
                    &sgr_mouse(child_raw),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route child press"),
                PointerRoute::Child(bytes) if bytes == child_raw
            ));

            surface.surface.modes = TerminalModes::default();
            let gutter = layout.gutter_column.expect("main layout gutter");
            assert!(matches!(
                route_pointer(
                    &sgr_mouse(format!("\x1b[<0;{gutter};2M").as_bytes()),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route gutter press"),
                PointerRoute::Viewport(_)
            ));
            assert!(viewport.gutter_drag_active());
            assert!(matches!(
                route_pointer(
                    &sgr_mouse(b"\x1b[<32;1;1M"),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route captured gutter motion"),
                PointerRoute::Viewport(_)
            ));
        }

        #[test]
        fn pointer_router_preserves_modified_native_selection_and_cancelled_drag_capture() {
            let physical = TerminalSize::new(2, 7);
            let layout = ChromeLayout::new(physical, false, ActiveScreen::Main);
            let snapshot = test_snapshot(layout.child, ActiveScreen::Main, Revision::new(3));
            let surface =
                AttachmentSurface::from_snapshot(&snapshot).expect("valid pointer surface");
            let mut viewport =
                ViewportController::with_layout(layout, snapshot.surface.scroll_metrics);
            let mut selection = SelectionController::default();

            assert!(matches!(
                route_pointer(
                    &sgr_mouse(b"\x1b[<4;1;1M"),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route modified native selection"),
                PointerRoute::Ignore
            ));
            assert!(matches!(
                route_pointer(
                    &sgr_mouse(b"\x1b[<0;1;1M"),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route cancellable selection press"),
                PointerRoute::SelectionChanged
            ));
            selection.reconcile(None);
            assert!(selection.owns_pointer_sequence());
            assert!(matches!(
                route_pointer(
                    &sgr_mouse(b"\x1b[<0;1;1m"),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route cancelled selection release"),
                PointerRoute::SelectionChanged
            ));
            assert!(!selection.owns_pointer_sequence());
        }

        #[test]
        fn pointer_router_invalidates_a_pinned_selection_before_history_navigation() {
            let physical = TerminalSize::new(3, 8);
            let layout = ChromeLayout::new(physical, false, ActiveScreen::Main);
            let snapshot = test_snapshot(layout.child, ActiveScreen::Main, Revision::new(7));
            let surface =
                AttachmentSurface::from_snapshot(&snapshot).expect("valid history surface");
            let mut viewport =
                ViewportController::with_layout(layout, snapshot.surface.scroll_metrics);
            let ViewportEffect::RequestHistoryWindow(query) = viewport.navigate(true, 1) else {
                panic!("initial history navigation requests a window");
            };
            let shape = query
                .response_shape(query.anchor)
                .expect("valid query shape");
            let rows = (0..shape.row_count)
                .map(|index| {
                    test_row(
                        query.anchor.viewport.columns,
                        &format!("history-{index}"),
                        TerminalStyle::default(),
                    )
                })
                .collect();
            assert!(matches!(
                viewport
                    .apply_view_history_window(TerminalSurfaceHistoryWindowResult::Frame(
                        TerminalSurfaceHistoryWindowFrame {
                            disposition: shape.disposition,
                            anchor: query.anchor,
                            target_offset_from_bottom: shape.target_offset_from_bottom,
                            first_row_from_live_top: shape.first_row_from_live_top,
                            rows,
                        },
                    ))
                    .expect("install history fixture"),
                ViewportEffect::Render | ViewportEffect::RenderAndRequestHistoryWindow(_)
            ));
            viewport.observe_presentation();

            let source = viewport
                .selection_source_identity(&surface)
                .expect("presented history identity");
            let rows = viewport
                .selection_rows(&surface)
                .expect("presented history rows");
            let mut selection = SelectionController::default();
            selection
                .begin(source, TerminalTextPoint::new(0, 0), rows)
                .expect("begin history selection");
            selection
                .update(source, TerminalTextPoint::new(0, 1), rows)
                .expect("extend history selection");
            selection.finish();
            assert!(selection.is_finalized());

            assert!(matches!(
                route_pointer(
                    &sgr_mouse(b"\x1b[<64;2;2M"),
                    &mut viewport,
                    &surface,
                    &mut selection,
                    true,
                )
                .expect("route pinned history wheel"),
                PointerRoute::Viewport(_)
            ));
            assert!(
                !selection.is_finalized(),
                "viewport navigation must retire coordinates from the previously presented slice"
            );
        }

        #[test]
        fn generic_nested_tui_wheel_update_preserves_the_styled_rightmost_cell() {
            let size = TerminalSize::new(1, 5);
            let child_mouse = TerminalModes {
                mouse_mode: TerminalMouseMode::PressRelease,
                mouse_encoding: TerminalMouseEncoding::Sgr,
                ..TerminalModes::default()
            };
            let mut snapshot = test_snapshot(size, ActiveScreen::Alternate, Revision::new(2));
            snapshot.surface.modes = child_mouse;
            snapshot.surface.rows[0].cells[4] = TerminalCell {
                contents: "x".to_owned(),
                style: TerminalStyle {
                    foreground: TerminalColor::Indexed(2),
                    ..TerminalStyle::default()
                },
                ..TerminalCell::default()
            };
            let viewport = ViewportController::with_layout(
                ChromeLayout::new(size, false, ActiveScreen::Alternate),
                None,
            );
            let status = StatusRenderer::new(None, size);
            let initial = ComposedFrame::compose(
                &snapshot.surface,
                None,
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("compose generic nested TUI frame");
            let mut presenter = DesktopPresenter::default();
            let mut output = ViewportFrameWriter::default();
            presenter
                .present(&mut output, initial, None)
                .expect("present initial right-edge cell");

            let wheel_raw = b"\x1b[<64;5;1M";
            let Some(HostInputEvent::Mouse(wheel)) = HostInputCodec::new()
                .feed(wheel_raw)
                .expect("decode generic nested TUI wheel")
                .into_iter()
                .next()
            else {
                panic!("wheel must decode as one mouse report");
            };
            assert!(!history_owns_gestures(ActiveScreen::Alternate, child_mouse));
            assert_eq!(
                route_mouse_to_child(&wheel, ActiveScreen::Alternate, child_mouse),
                Some(wheel_raw.to_vec())
            );

            snapshot.surface.rows[0].cells[4].contents = "y".to_owned();
            snapshot.surface.rows[0].cells[4].style.background = TerminalColor::Rgb(1, 2, 3);
            let updated = ComposedFrame::compose(
                &snapshot.surface,
                presenter.baseline.as_ref(),
                &viewport,
                &status,
                TerminalViewTransportState::Active,
            )
            .expect("compose child-owned wheel update");
            output = ViewportFrameWriter::default();
            presenter
                .present(&mut output, updated, None)
                .expect("present child-owned wheel update");
            assert_eq!((output.writes, output.flushes), (1, 1));
            assert!(find_bytes(&output.bytes, b"\x1b[1;5H").is_some());
            assert!(output.bytes.contains(&b'y'));
            assert!(find_bytes(&output.bytes, b"\x1b[K").is_none());
            assert!(find_bytes(&output.bytes, b"\x1b[2K").is_none());
        }

        #[test]
        fn session_end_projects_fixed_content_free_normal_mode_diagnostics() {
            let natural = terminal_end_completion(TerminalViewEndReason::NaturalExit)
                .expect("natural end is a successful terminal completion");
            assert_eq!(
                natural,
                TerminalCompletion::SessionEnded(TerminalViewEndReason::NaturalExit)
            );
            let diagnostic = completion_diagnostic(natural)
                .expect("natural diagnostic")
                .expect("natural end is visible");
            assert!(!diagnostic.contains(&b'\x1b'));
            assert!(!diagnostic.windows(4).any(|window| window == b"cwd="));
            assert!(!diagnostic.windows(6).any(|window| window == b"route="));
            assert!(!diagnostic.windows(7).any(|window| window == b"ticket="));

            let error = terminal_end_completion(TerminalViewEndReason::DriverFailure)
                .expect_err("driver failure remains a typed CLI error");
            assert!(matches!(error, CliError::TerminalDriverFailure));
            assert_eq!(error.to_string(), "the daemon-owned terminal driver failed");
            assert!(matches!(
                completion_diagnostic(TerminalCompletion::SessionEnded(
                    TerminalViewEndReason::DriverFailure
                )),
                Err(CliError::TerminalDriverFailure)
            ));
        }

        #[test]
        fn created_session_attach_failure_retains_the_exact_live_session_identity() {
            let session_id = zterm_core::SessionId::from_array([0xc1; 16]);
            let source = DaemonError::new(
                DomainErrorKind::SessionOccupied,
                "the exact created Session could not be attached",
            );
            let error = preserve_created_session::<()>(session_id, Err(source))
                .expect_err("attach failure preserves the committed create");
            assert!(matches!(
                error,
                CliError::CreatedSessionAttach {
                    session_id: actual,
                    source,
                } if actual == session_id && source.kind() == DomainErrorKind::SessionOccupied
            ));
        }

        #[tokio::test(flavor = "current_thread")]
        async fn cancelled_stateful_prepare_preserves_every_exact_terminal_result() {
            let session_id = zterm_core::SessionId::from_array([0xc2; 16]);
            let dropped = Arc::new(AtomicBool::new(false));
            let (submitted_sender, submitted_receiver) = tokio::sync::oneshot::channel();
            let (release_sender, release_receiver) = tokio::sync::oneshot::channel();
            let drop_probe = FutureDropProbe(Arc::clone(&dropped));
            let mut operation = Box::pin(async move {
                let _ = submitted_sender.send(());
                let _ = release_receiver.await;
                drop(drop_probe);
                Ok::<SessionId, CliError>(session_id)
            });
            poll_once_pending(operation.as_mut()).await;
            submitted_receiver
                .await
                .expect("stateful operation crossed its submission barrier");

            let mut finishing = Box::pin(finish_submitted_after_cancellation(
                operation.as_mut(),
                InactiveCancellation::LocalDetach,
            ));
            poll_once_pending(finishing.as_mut()).await;
            assert!(
                !dropped.load(Ordering::SeqCst),
                "cancellation must retain the submitted operation future"
            );
            release_sender
                .send(())
                .expect("release exact committed result");
            let exact = finishing.await.expect("exact committed result");
            assert!(matches!(
                exact,
                InactiveWait::CompletedAfterCancellation {
                    value,
                    cancellation: InactiveCancellation::LocalDetach,
                } if value == session_id
            ));
            assert!(dropped.load(Ordering::SeqCst));

            let created_error = CliError::CreatedSessionAttach {
                session_id,
                source: DaemonError::new(
                    DomainErrorKind::SessionOccupied,
                    "the exact created Session remains live",
                ),
            };
            let created_error = cancelled_submitted_error(created_error).await;
            assert!(matches!(
                created_error,
                CliError::CreatedSessionAttach {
                    session_id: actual,
                    source,
                } if actual == session_id && source.kind() == DomainErrorKind::SessionOccupied
            ));

            let unknown = cancelled_submitted_error(CliError::Daemon(DaemonError::new(
                DomainErrorKind::OperationOutcomeUnknown,
                "the submitted Session result could not be proven",
            )))
            .await;
            assert!(matches!(
                unknown,
                CliError::Daemon(error)
                    if error.kind() == DomainErrorKind::OperationOutcomeUnknown
            ));
        }

        #[test]
        fn cancelled_prepared_session_identity_is_user_visible_and_truthful() {
            let session_id = zterm_core::SessionId::from_array([0xc3; 16]);
            let completion =
                inactive_cancellation_result(InactiveCancellation::LocalDetach, Some(session_id))
                    .expect("prefix detach remains a successful completion");
            let diagnostic = completion_diagnostic(completion)
                .expect("safe completion projection")
                .expect("prepared detach is user-visible");
            let diagnostic = String::from_utf8(diagnostic).expect("ASCII completion diagnostic");
            assert!(diagnostic.contains(&session_id.to_string()));
            assert!(diagnostic.contains("remains live"));

            let signal = inactive_cancellation_result(
                InactiveCancellation::Signal(TerminalSignalCancellation::new("SIGTERM", Some(()))),
                Some(session_id),
            )
            .expect_err("external signal retains cancelled semantics");
            assert!(matches!(
                signal,
                CliError::Daemon(ref error) if error.kind() == DomainErrorKind::Cancelled
            ));
            assert!(signal.to_string().contains(&session_id.to_string()));
            assert!(signal.to_string().contains("remains live"));
        }

        struct FutureDropProbe(Arc<AtomicBool>);

        impl Drop for FutureDropProbe {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        async fn cancelled_submitted_error(error: CliError) -> CliError {
            let dropped = Arc::new(AtomicBool::new(false));
            let (submitted_sender, submitted_receiver) = tokio::sync::oneshot::channel();
            let (release_sender, release_receiver) = tokio::sync::oneshot::channel();
            let drop_probe = FutureDropProbe(Arc::clone(&dropped));
            let mut operation = Box::pin(async move {
                let _ = submitted_sender.send(());
                let _ = release_receiver.await;
                drop(drop_probe);
                Err::<SessionId, CliError>(error)
            });
            poll_once_pending(operation.as_mut()).await;
            submitted_receiver
                .await
                .expect("error operation crossed its submission barrier");

            let mut finishing = Box::pin(finish_submitted_after_cancellation(
                operation.as_mut(),
                InactiveCancellation::LocalDetach,
            ));
            poll_once_pending(finishing.as_mut()).await;
            assert!(!dropped.load(Ordering::SeqCst));
            release_sender
                .send(())
                .expect("release exact terminal error");
            let result = finishing.await;
            assert!(dropped.load(Ordering::SeqCst));
            match result {
                Err(error) => error,
                Ok(_) => panic!("submitted error unexpectedly became success"),
            }
        }

        async fn poll_once_pending<F: Future>(mut future: Pin<&mut F>) {
            std::future::poll_fn(|context| match future.as_mut().poll(context) {
                Poll::Pending => Poll::Ready(()),
                Poll::Ready(_) => panic!("barrier future completed before its release"),
            })
            .await;
        }

        #[test]
        fn terminal_guard_restores_attributes_on_success_error_cancel_and_panic() {
            for path in [
                GuardPath::Success,
                GuardPath::Error,
                GuardPath::Cancel,
                GuardPath::Panic,
            ] {
                let pty = openpty(None, None).expect("open PTY");
                let original = tcgetattr(&pty.slave).expect("original attributes");
                match path {
                    GuardPath::Success => {
                        let mut guard =
                            TerminalGuard::enter(&pty.slave, &pty.slave).expect("guard");
                        assert_descriptor_cloexec(&guard.input);
                        assert_descriptor_cloexec(&guard.output);
                        guard.restore().expect("restore");
                    }
                    GuardPath::Error => {
                        fn fail(input: &OwnedFd) -> Result<(), CliError> {
                            let _guard = TerminalGuard::enter(input, input)?;
                            Err(CliError::Io("fixture error".to_owned()))
                        }
                        assert!(fail(&pty.slave).is_err());
                    }
                    GuardPath::Cancel => {
                        let guard = TerminalGuard::enter(&pty.slave, &pty.slave).expect("guard");
                        drop(guard);
                    }
                    GuardPath::Panic => {
                        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                            let _guard =
                                TerminalGuard::enter(&pty.slave, &pty.slave).expect("guard");
                            panic!("fixture panic");
                        }));
                        assert!(result.is_err());
                    }
                }
                assert_restored_termios(
                    tcgetattr(&pty.slave).expect("restored attributes"),
                    original,
                );
                let mut master = File::from(pty.master);
                let mut cleanup = vec![0_u8; ENTER_TERMINAL_UI.len() + RESTORE_TERMINAL_UI.len()];
                master.read_exact(&mut cleanup).expect("terminal UI bytes");
                assert_eq!(cleanup, [ENTER_TERMINAL_UI, RESTORE_TERMINAL_UI].concat());
            }
        }

        #[tokio::test]
        async fn task_abort_restores_termios_and_terminal_ui_after_an_entry_barrier() {
            let pty = openpty(None, None).expect("open cancellation PTY");
            let mut master = File::from(pty.master);
            let slave = File::from(pty.slave);
            let probe = slave.try_clone().expect("cancellation termios probe");
            let original = tcgetattr(&probe).expect("cancellation original attributes");
            let task_terminal = slave.try_clone().expect("cancellation task terminal");
            let (entered_sender, entered_receiver) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let _guard =
                    TerminalGuard::enter(&task_terminal, &task_terminal).expect("task guard");
                let _ = entered_sender.send(());
                std::future::pending::<()>().await;
            });

            entered_receiver
                .await
                .expect("task crossed the raw-terminal entry barrier");
            task.abort();
            assert!(
                task.await
                    .expect_err("aborted terminal task")
                    .is_cancelled(),
                "the task must end through Tokio cancellation"
            );
            assert_restored_termios(
                tcgetattr(&probe).expect("task-cancel restored attributes"),
                original,
            );
            let mut cleanup = vec![0_u8; ENTER_TERMINAL_UI.len() + RESTORE_TERMINAL_UI.len()];
            master
                .read_exact(&mut cleanup)
                .expect("task-cancel terminal UI bytes");
            assert_eq!(cleanup, [ENTER_TERMINAL_UI, RESTORE_TERMINAL_UI].concat());
        }

        #[cfg(not(target_os = "macos"))]
        fn assert_restored_termios(actual: Termios, expected: Termios) {
            assert_eq!(actual, expected);
        }

        #[cfg(target_os = "macos")]
        fn assert_restored_termios(actual: Termios, expected: Termios) {
            // The macOS PTY kernel may maintain PENDIN independently when
            // attributes are read back. Production still restores the exact
            // saved Termios object with TCSANOW; only this assertion ignores
            // that one kernel-owned bit while comparing every stable field.
            assert_eq!(actual.input_flags, expected.input_flags);
            assert_eq!(actual.output_flags, expected.output_flags);
            assert_eq!(actual.control_flags, expected.control_flags);
            assert_eq!(
                actual.local_flags - LocalFlags::PENDIN,
                expected.local_flags - LocalFlags::PENDIN
            );
            assert_eq!(actual.control_chars, expected.control_chars);
            assert_eq!(cfgetispeed(&actual), cfgetispeed(&expected));
            assert_eq!(cfgetospeed(&actual), cfgetospeed(&expected));
        }

        #[derive(Clone, Copy)]
        enum GuardPath {
            Success,
            Error,
            Cancel,
            Panic,
        }

        #[test]
        fn stdin_pump_wakes_and_joins_without_changing_stdin_status_flags() {
            let pty = openpty(None, None).expect("open PTY");
            let original = OFlag::from_bits_truncate(
                fcntl(pty.slave.as_raw_fd(), FcntlArg::F_GETFL).expect("original flags"),
            );
            let mut pump = StdinPump::start(&pty.slave, InputEpoch::new()).expect("stdin pump");
            assert_descriptor_cloexec(&pump._cancellation_read_guard);
            assert_descriptor_cloexec(
                pump.cancellation_write
                    .as_ref()
                    .expect("cancellation writer"),
            );
            let while_running = OFlag::from_bits_truncate(
                fcntl(pty.slave.as_raw_fd(), FcntlArg::F_GETFL).expect("running flags"),
            );
            assert_eq!(while_running, original);
            pump.shutdown().expect("pump shutdown");
            let after_join = OFlag::from_bits_truncate(
                fcntl(pty.slave.as_raw_fd(), FcntlArg::F_GETFL).expect("joined flags"),
            );
            assert_eq!(after_join, original);
        }

        fn assert_descriptor_cloexec(fd: &impl AsFd) {
            let flags = rustix::io::fcntl_getfd(fd).expect("descriptor flags");
            assert!(
                flags.contains(rustix::io::FdFlags::CLOEXEC),
                "owned terminal descriptors must not survive exec"
            );
        }

        #[test]
        fn signals_restore_termios_and_ui_before_the_normal_mode_diagnostic() {
            if std::env::var_os("ZTERM_TERMINAL_SIGNAL_CHILD").is_some() {
                return;
            }
            for signal in [NixSignal::SIGINT, NixSignal::SIGTERM, NixSignal::SIGHUP] {
                assert_signal_restoration(signal);
            }
        }

        fn assert_signal_restoration(signal: NixSignal) {
            let pty = openpty(None, None).expect("open signal PTY");
            let master = File::from(pty.master);
            let slave = File::from(pty.slave);
            let probe = slave.try_clone().expect("signal termios probe");
            let original = tcgetattr(&probe).expect("signal original termios");
            let child_stdin = slave.try_clone().expect("signal child stdin");
            let child_stdout = slave.try_clone().expect("signal child stdout");
            let child_stderr = slave.try_clone().expect("signal child stderr");
            let mut child = Command::new(std::env::current_exe().expect("current test binary"))
                .arg("--exact")
                .arg("terminal_ui::unix::tests::signal_restore_child")
                .arg("--ignored")
                .arg("--nocapture")
                .env("ZTERM_TERMINAL_SIGNAL_CHILD", "1")
                .stdin(Stdio::from(child_stdin))
                .stdout(Stdio::from(child_stdout))
                .stderr(Stdio::from(child_stderr))
                .spawn()
                .expect("spawn isolated signal child");
            drop(slave);

            let (entered_sender, entered_receiver) = mpsc::channel();
            let reader = std::thread::spawn(move || {
                let mut master = master;
                let mut bytes = Vec::new();
                let mut entered = false;
                let mut buffer = [0_u8; 1024];
                loop {
                    match master.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => {
                            bytes.extend_from_slice(&buffer[..read]);
                            if !entered && contains_bytes(&bytes, ENTER_TERMINAL_UI) {
                                entered = true;
                                let _ = entered_sender.send(());
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(error) if linux_pty_master_closed(&error) => break,
                        Err(error) => panic!("read signal child PTY: {error}"),
                    }
                }
                bytes
            });

            if entered_receiver
                .recv_timeout(Duration::from_secs(5))
                .is_err()
            {
                let _ = child.kill();
                let _ = child.wait();
                drop(probe);
                let _ = reader.join();
                panic!("signal child did not enter raw terminal mode");
            }
            kill(Pid::from_raw(child.id() as i32), signal).expect("send terminal signal");
            let status = wait_for_signal_child(&mut child);
            assert_restored_termios(
                tcgetattr(&probe).expect("signal restored termios"),
                original,
            );
            drop(probe);
            let bytes = reader.join().expect("signal PTY reader");
            assert!(status.success(), "isolated signal child failed: {signal:?}");
            let restore =
                find_bytes(&bytes, RESTORE_TERMINAL_UI).expect("signal cleanup bytes are written");
            let diagnostic = find_bytes(&bytes, b"ZTERM_SAFE_SIGNAL_DIAGNOSTIC")
                .expect("normal-mode signal diagnostic is written");
            assert!(
                restore < diagnostic,
                "terminal cleanup must precede the normal-mode diagnostic"
            );
        }

        fn wait_for_signal_child(child: &mut std::process::Child) -> std::process::ExitStatus {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(status) = child.try_wait().expect("poll signal child") {
                    return status;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let status = child.wait().expect("reap timed-out signal child");
                    panic!("signal child did not exit before its deadline: {status}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
            find_bytes(haystack, needle).is_some()
        }

        fn linux_pty_master_closed(error: &io::Error) -> bool {
            cfg!(target_os = "linux") && error.raw_os_error() == Some(nix::errno::Errno::EIO as i32)
        }

        fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
            haystack
                .windows(needle.len())
                .position(|window| window == needle)
        }

        #[test]
        #[ignore = "isolated terminal-signal helper"]
        fn signal_restore_child() {
            if std::env::var_os("ZTERM_TERMINAL_SIGNAL_CHILD").is_none() {
                return;
            }
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("signal child runtime");
            let result = runtime.block_on(async {
                let TerminalSignals {
                    resize: _resize,
                    mut interrupt,
                    mut terminate,
                    mut hangup,
                } = TerminalSignals::install()?;
                let stdin = io::stdin();
                let stdout = io::stdout();
                let mut guard = TerminalGuard::enter(&stdin, &stdout)?;
                let (cancellation_sender, mut cancellation_receiver) = watch::channel(None);
                let result = select_terminal_termination(
                    async move {
                        let cancellation =
                            receive_terminal_cancellation(&mut cancellation_receiver).await;
                        Err(cancellation.error(None))
                    },
                    &mut interrupt,
                    &mut terminate,
                    &mut hangup,
                    cancellation_sender,
                )
                .await;
                guard.restore()?;
                result
            });
            let error = result.expect_err("signal interrupts the guarded view");
            assert!(matches!(
                error,
                CliError::Daemon(ref error) if error.kind() == DomainErrorKind::Cancelled
            ));
            write_all_fd(&io::stderr(), b"ZTERM_SAFE_SIGNAL_DIAGNOSTIC")
                .expect("write normal-mode diagnostic marker");
        }

        #[test]
        fn panic_restores_termios_and_ui_before_the_normal_mode_diagnostic() {
            if std::env::var_os("ZTERM_TERMINAL_PANIC_CHILD").is_some() {
                return;
            }
            let pty = openpty(None, None).expect("open panic PTY");
            let master = File::from(pty.master);
            let slave = File::from(pty.slave);
            let probe = slave.try_clone().expect("panic termios probe");
            let original = tcgetattr(&probe).expect("panic original termios");
            let mut child = Command::new(std::env::current_exe().expect("current test binary"))
                .arg("--exact")
                .arg("terminal_ui::unix::tests::panic_restore_child")
                .arg("--ignored")
                .arg("--nocapture")
                .env("ZTERM_TERMINAL_PANIC_CHILD", "1")
                .stdin(Stdio::from(slave.try_clone().expect("panic child stdin")))
                .stdout(Stdio::from(slave.try_clone().expect("panic child stdout")))
                .stderr(Stdio::from(slave.try_clone().expect("panic child stderr")))
                .spawn()
                .expect("spawn isolated panic child");
            drop(slave);

            let reader = std::thread::spawn(move || {
                let mut master = master;
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 1024];
                loop {
                    match master.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => bytes.extend_from_slice(&buffer[..read]),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(error) if linux_pty_master_closed(&error) => break,
                        Err(error) => panic!("read panic child PTY: {error}"),
                    }
                }
                bytes
            });
            let status = wait_for_signal_child(&mut child);
            assert_restored_termios(tcgetattr(&probe).expect("panic restored termios"), original);
            drop(probe);
            let bytes = reader.join().expect("panic PTY reader");
            assert!(status.success(), "isolated panic child failed");
            let restore =
                find_bytes(&bytes, RESTORE_TERMINAL_UI).expect("panic cleanup bytes are written");
            let diagnostic = find_bytes(&bytes, b"ZTERM_SAFE_PANIC_DIAGNOSTIC")
                .expect("normal-mode panic diagnostic is written");
            assert!(
                restore < diagnostic,
                "terminal cleanup must precede the panic diagnostic"
            );
        }

        #[test]
        #[ignore = "isolated terminal-panic helper"]
        fn panic_restore_child() {
            if std::env::var_os("ZTERM_TERMINAL_PANIC_CHILD").is_none() {
                return;
            }
            let panic_hook = ScopedPanicHook::suppress();
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let stdin = io::stdin();
                let stdout = io::stdout();
                let _guard = TerminalGuard::enter(&stdin, &stdout).expect("panic child guard");
                panic!("redacted terminal fixture panic");
            }));
            assert!(result.is_err());
            drop(panic_hook);
            write_all_fd(&io::stderr(), b"ZTERM_SAFE_PANIC_DIAGNOSTIC")
                .expect("write normal-mode panic diagnostic marker");
        }

        #[test]
        fn panic_hook_restoration_is_proved_in_an_isolated_process() {
            if std::env::var_os("ZTERM_PANIC_HOOK_CHILD").is_some() {
                return;
            }
            let status = Command::new(std::env::current_exe().expect("current test binary"))
                .arg("--exact")
                .arg("terminal_ui::unix::tests::panic_hook_child")
                .arg("--ignored")
                .env("ZTERM_PANIC_HOOK_CHILD", "1")
                .status()
                .expect("run isolated panic-hook child");
            assert!(status.success());
        }

        #[test]
        #[ignore = "isolated panic-hook helper"]
        fn panic_hook_child() {
            if std::env::var_os("ZTERM_PANIC_HOOK_CHILD").is_none() {
                return;
            }
            static CALLS: AtomicUsize = AtomicUsize::new(0);
            let process_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {
                CALLS.fetch_add(1, Ordering::SeqCst);
            }));
            {
                let _suppressed = ScopedPanicHook::suppress();
                let result = std::panic::catch_unwind(|| panic!("redacted fixture panic"));
                assert!(result.is_err());
            }
            let result = std::panic::catch_unwind(|| panic!("post-restore fixture panic"));
            assert!(result.is_err());
            assert_eq!(CALLS.load(Ordering::SeqCst), 1);
            let custom = std::panic::take_hook();
            drop(custom);
            std::panic::set_hook(process_hook);
        }
    }
}
