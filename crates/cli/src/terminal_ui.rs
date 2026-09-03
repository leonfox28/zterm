//! Unix raw-terminal ownership and daemon-authored ANSI rendering.

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
    #[cfg(test)]
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
    use unicode_width::UnicodeWidthChar;
    use zterm_core::terminal::{
        ALTERNATE_SCREEN_SELECTION_ANSI, ActiveScreen, MAIN_SCREEN_SELECTION_ANSI,
        MAX_HISTORY_PAGE_ROWS, TerminalHistoryCursor, TerminalHistoryDirection,
        TerminalHistoryPage, TerminalHistoryResult, TerminalHistoryWindowAnchor,
        TerminalHistoryWindowQuery, TerminalHistoryWindowResult, TerminalModes,
        TerminalMouseEncoding, TerminalMouseMode, TerminalScrollAction, TerminalScrollMetrics,
        TerminalSize, TerminalViewportDisposition, TerminalViewportFrame, TerminalViewportResult,
    };
    use zterm_core::viewport_cache::{CachedViewportWindow, ViewportCache, ViewportCacheUpdate};
    use zterm_core::{DomainErrorKind, RESERVED_DEVICE_ALIAS, ResourceLimits, Revision, SessionId};
    use zterm_daemon::error::DaemonError;
    use zterm_daemon::operations::{
        LocalRuntime, PreparedTerminalView, TerminalViewConnectionPath,
        TerminalViewConnectionStatus, TerminalViewDelta, TerminalViewEndReason, TerminalViewEvent,
        TerminalViewSnapshot, TerminalViewTransportState,
    };

    use super::super::{CliError, TerminalRequest, TerminalRequestKind};

    const STDIN_CHANNEL_CAPACITY: usize = 8;
    const STDIN_CHUNK_BYTES: usize = 4 * 1024;
    const CONTROL_PREFIX_TIMEOUT: Duration = Duration::from_secs(1);
    const DETACH_TIMEOUT: Duration = Duration::from_secs(2);
    const DRAG_REQUEST_INTERVAL: Duration = Duration::from_millis(33);
    const MIN_VIEWPORT_PRESENT_INTERVAL: Duration = Duration::from_millis(16);
    const HOST_SYNC_BEGIN: &[u8] = b"\x1b[?2026h";
    const HOST_SYNC_END: &[u8] = b"\x1b[?2026l";
    const HOST_INPUT_CAPTURE: &[u8] = b"\x1b[?1003h\x1b[?1006h";
    const HISTORY_LOADING_NOTICE: &str = "[zterm: loading retained history]";
    const ENTER_TERMINAL_UI: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[?1003h\x1b[?1006h";
    const RECONNECTING_STATUS: &[u8] = b"\r\n[zterm: reconnecting]\r\n";
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
            prepared.initial_snapshot().active_screen(),
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

        let mut renderer = TerminalRenderer::new();
        let initial_scroll_metrics = prepared.initial_snapshot().scroll_metrics();
        let mut viewport = ViewportController::with_layout(latest_layout, initial_scroll_metrics);
        let mut status_renderer = StatusRenderer::new(remote_alias, physical_size);
        render_snapshot_stdout(
            &mut renderer,
            prepared.initial_snapshot(),
            &viewport,
            &mut status_renderer,
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
            // and let the first real gesture retry or negotiate fallback.
            viewport.window_cache.defer_pending_request();
        }
        let mut sync_requested = false;
        let mut input_codec = HostInputCodec::new();
        let mut deferred_active = false;

        let loop_result = 'terminal: loop {
            let now = Instant::now();
            if viewport_pacer.due(now) {
                if let Err(error) = present_cached_viewport_stdout(
                    &mut viewport,
                    &mut status_renderer,
                    transport_state,
                    &mut viewport_pacer,
                    now,
                    false,
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
                            &writer,
                            renderer.revision(),
                            &mut status_renderer,
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
                        viewport.effective_screen(renderer.active_screen()),
                    );
                    let latest = layout.child;
                    viewport_pacer.cancel();
                    viewport.set_layout(layout);
                    status_renderer.resize(latest_physical);
                    if let Err(error) = render_view_stdout(
                        &viewport,
                        &mut status_renderer,
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
                                &writer,
                                renderer.revision(),
                                &mut status_renderer,
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
                        &mut viewport,
                        &mut status_renderer,
                        transport_state,
                        &mut viewport_pacer,
                        now,
                        false,
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
                            if let Err(error) = render_view_stdout(
                                &viewport,
                                &mut status_renderer,
                                transport_state,
                            ) {
                                break Err(error);
                            }
                            viewport.observe_presentation();
                            viewport_pacer.mark_presented(Instant::now());
                        }
                        TerminalViewEvent::Snapshot(snapshot) => {
                            viewport_pacer.cancel();
                            let pinned_history = viewport.is_history();
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
                            viewport.observe_snapshot(snapshot.scroll_metrics());
                            let layout = ChromeLayout::new(
                                physical_size,
                                remote_request,
                                viewport.effective_screen(snapshot.active_screen()),
                            );
                            viewport.set_layout(layout);
                            let _ = resize_coalescer.observe(layout.child, transport_state);
                            let history_refill = viewport.refetch_history_window();
                            let rendered = if pinned_history {
                                renderer
                                    .observe_snapshot((&snapshot).into())
                                    .and_then(|()| {
                                        render_history_stdout(
                                            &viewport,
                                            &mut status_renderer,
                                            transport_state,
                                        )
                                    })
                            } else {
                                render_snapshot_stdout(
                                    &mut renderer,
                                    &snapshot,
                                    &viewport,
                                    &mut status_renderer,
                                    transport_state,
                                )
                            };
                            if let Err(error) = rendered {
                                break Err(error);
                            }
                            viewport.observe_presentation();
                            viewport_pacer.mark_presented(Instant::now());
                            prefix.clear_pending();
                            sync_requested = false;
                            if let Err(error) = writer.snapshot_applied(snapshot.revision()).await {
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
                            let delta_result = if rendered_live {
                                if renderer.revision() != delta.from_revision() {
                                    Ok(DeltaRender::Gap)
                                } else {
                                    viewport.observe_live_metrics(delta.scroll_metrics());
                                    let layout = ChromeLayout::new(
                                        physical_size,
                                        remote_request,
                                        delta.active_screen(),
                                    );
                                    viewport.set_layout(layout);
                                    render_delta_stdout(
                                        &mut renderer,
                                        &delta,
                                        &viewport,
                                        &mut status_renderer,
                                        transport_state,
                                    )
                                }
                            } else {
                                renderer.observe_delta((&delta).into())
                            };
                            match delta_result {
                                Ok(DeltaRender::Applied) => {
                                    viewport.observe_live_metrics(delta.scroll_metrics());
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
                                            .snapshot_applied(delta.to_revision())
                                            .await
                                    {
                                        break Err(error.into());
                                    }
                                }
                                Ok(DeltaRender::Gap) => {
                                    viewport_pacer.cancel();
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
                                            .request_sync(renderer.revision())
                                            .await
                                        {
                                            break 'terminal Err(error.into());
                                        }
                                    }
                                }
                                Err(error) => break Err(error),
                            }
                        }
                        TerminalViewEvent::History(result) => {
                            viewport_pacer.cancel();
                            viewport.apply_history(result)?;
                            if let Err(error) = render_history_stdout(
                                &viewport,
                                &mut status_renderer,
                                transport_state,
                            ) {
                                break Err(error);
                            }
                            viewport.observe_presentation();
                            viewport_pacer.mark_presented(Instant::now());
                        }
                        TerminalViewEvent::Viewport(result) => {
                            let effect = viewport.apply_viewport(result)?;
                            if apply_viewport_effect(
                                effect,
                                &mut viewport,
                                &writer,
                                renderer.revision(),
                                &mut status_renderer,
                                transport_state,
                                &mut viewport_pacer,
                                false,
                            )
                            .await?
                            {
                                sync_requested = true;
                            }
                        }
                        TerminalViewEvent::HistoryWindow(result) => {
                            let effect = viewport.apply_history_window(result)?;
                            if apply_viewport_effect(
                                effect,
                                &mut viewport,
                                &writer,
                                renderer.revision(),
                                &mut status_renderer,
                                transport_state,
                                &mut viewport_pacer,
                                false,
                            )
                            .await?
                            {
                                sync_requested = true;
                            }
                        }
                        TerminalViewEvent::SyncRequired { .. } => {
                            viewport_pacer.cancel();
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
                            let host_events = match input_codec.feed(&bytes) {
                                Ok(events) => events,
                                Err(error) => break 'terminal Err(error),
                            };
                            let mut force_viewport_presentation = false;
                            for host_event in host_events {
                                match host_event {
                                    HostInputEvent::Bytes(bytes) => {
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
                                                        &writer,
                                                        renderer.revision(),
                                                        &mut status_renderer,
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
                                    HostInputEvent::Paste(bytes) => {
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
                                                &writer,
                                                renderer.revision(),
                                                &mut status_renderer,
                                                transport_state,
                                                &mut viewport_pacer,
                                                true,
                                            ).await? {
                                                sync_requested = true;
                                            }
                                        }
                                    }
                                    HostInputEvent::PageUp | HostInputEvent::PageDown => {
                                        let older = matches!(host_event, HostInputEvent::PageUp);
                                        let raw = if older { PAGE_UP } else { PAGE_DOWN };
                                        if viewport.is_resume_pending() {
                                            viewport.retain_resume_input(raw)?;
                                        } else if viewport.is_history()
                                            || live_history_navigation_allowed(transport_state)
                                                && history_owns_gestures(
                                                    renderer.active_screen(),
                                                    renderer.modes(),
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
                                                &writer,
                                                renderer.revision(),
                                                &mut status_renderer,
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
                                        if let Some(effect) = viewport.handle_gutter_mouse(
                                            &mouse,
                                            live_history_navigation_allowed(transport_state),
                                        ) {
                                            force_viewport_presentation |= mouse.release;
                                            if apply_viewport_effect(
                                                effect,
                                                &mut viewport,
                                                &writer,
                                                renderer.revision(),
                                                &mut status_renderer,
                                                transport_state,
                                                &mut viewport_pacer,
                                                true,
                                            ).await? {
                                                sync_requested = true;
                                            }
                                            continue;
                                        }
                                        if viewport.outside_layout(&mouse) {
                                            continue;
                                        }
                                        if viewport.is_history() && mouse.is_wheel() {
                                            let effect = viewport.navigate(mouse.wheel_is_up(), 1);
                                            if apply_viewport_effect(
                                                effect,
                                                &mut viewport,
                                                &writer,
                                                renderer.revision(),
                                                &mut status_renderer,
                                                transport_state,
                                                &mut viewport_pacer,
                                                true,
                                            ).await? {
                                                sync_requested = true;
                                            }
                                            continue;
                                        }
                                        let routed = route_mouse_to_child(
                                            &mouse,
                                            renderer.active_screen(),
                                            renderer.modes(),
                                        );
                                        match routed {
                                            Some(bytes) if viewport.is_resume_pending() => {
                                                viewport.retain_resume_input(&bytes)?;
                                            }
                                            Some(bytes) if viewport.is_live()
                                                && transport_state
                                                    == TerminalViewTransportState::Active =>
                                            {
                                                if let Err(error) = writer.write_input(bytes).await {
                                                    break 'terminal Err(error.into());
                                                }
                                            }
                                            Some(_) => {}
                                            None if viewport.is_live()
                                                && mouse.is_wheel()
                                                && live_history_navigation_allowed(transport_state)
                                                && history_owns_gestures(
                                                    renderer.active_screen(),
                                                    renderer.modes(),
                                                ) =>
                                            {
                                                let effect = viewport.navigate(mouse.wheel_is_up(), 1);
                                                if apply_viewport_effect(
                                                    effect,
                                                    &mut viewport,
                                                    &writer,
                                                    renderer.revision(),
                                                    &mut status_renderer,
                                                    transport_state,
                                                    &mut viewport_pacer,
                                                    true,
                                                ).await? {
                                                    sync_requested = true;
                                                }
                                            }
                                            None => {}
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
                                &mut viewport,
                                &mut status_renderer,
                                transport_state,
                                &mut viewport_pacer,
                                Instant::now(),
                                force_viewport_presentation,
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
        if next != previous && !status_renderer.enabled() {
            render_transport_state_stdout(next)?;
        }
        if render_transport_transition_view_stdout(
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

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct ChromeLayout {
        child: TerminalSize,
        gutter_column: Option<u16>,
        status_row: Option<u16>,
    }

    impl ChromeLayout {
        fn new(physical: TerminalSize, remote: bool, screen: ActiveScreen) -> Self {
            let limits = ResourceLimits::default();
            let status_row = (remote && physical.rows > 1).then_some(physical.rows);
            let status_rows = u16::from(status_row.is_some());
            let usable_rows = physical
                .rows
                .saturating_sub(status_rows)
                .min(limits.max_viewport_rows)
                .max(1);
            let usable_columns = physical.columns.min(limits.max_viewport_columns).max(1);
            let gutter_column =
                (screen == ActiveScreen::Main && usable_columns > 4).then_some(usable_columns);
            let child_columns = usable_columns.saturating_sub(u16::from(gutter_column.is_some()));
            Self {
                child: TerminalSize::new(usable_rows, child_columns.max(1)),
                gutter_column,
                status_row,
            }
        }
    }

    fn child_terminal_size(physical: TerminalSize, remote: bool) -> TerminalSize {
        ChromeLayout::new(physical, remote, ActiveScreen::Main).child
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct ScrollbarGeometry {
        track_rows: u16,
        thumb_top: u16,
        thumb_len: u16,
    }

    impl ScrollbarGeometry {
        fn new(track_rows: u16, metrics: TerminalScrollMetrics) -> Option<Self> {
            if track_rows == 0 || !metrics.is_valid() || metrics.max_offset_from_bottom == 0 {
                return None;
            }
            let track = u128::from(track_rows);
            let visible = u128::from(metrics.viewport_rows);
            let maximum = u128::from(metrics.max_offset_from_bottom);
            let thumb_len =
                ((track.checked_mul(visible)? / visible.checked_add(maximum)?).max(1)).min(track);
            let travel = track.saturating_sub(thumb_len);
            let newer_distance = maximum.saturating_sub(u128::from(
                metrics
                    .offset_from_bottom
                    .min(metrics.max_offset_from_bottom),
            ));
            let thumb_top = if travel == 0 {
                0
            } else {
                newer_distance
                    .checked_mul(travel)?
                    .checked_add(maximum / 2)?
                    / maximum
            };
            Some(Self {
                track_rows,
                thumb_top: u16::try_from(thumb_top.min(track)).ok()?,
                thumb_len: u16::try_from(thumb_len).ok()?,
            })
        }

        fn contains_thumb(self, row: u16) -> bool {
            let row = row.saturating_sub(1);
            row >= self.thumb_top && row < self.thumb_top.saturating_add(self.thumb_len)
        }

        fn grab_row(self, row: u16) -> u16 {
            row.saturating_sub(1)
                .saturating_sub(self.thumb_top)
                .min(self.thumb_len.saturating_sub(1))
        }

        fn offset_for_pointer(self, row: u16, grab_row: u16, maximum_offset: u64) -> u64 {
            if maximum_offset == 0 {
                return 0;
            }
            let travel = self.track_rows.saturating_sub(self.thumb_len);
            if travel == 0 {
                return maximum_offset;
            }
            let pointer = row.saturating_sub(1).min(self.track_rows.saturating_sub(1));
            let top = pointer
                .saturating_sub(grab_row.min(self.thumb_len.saturating_sub(1)))
                .min(travel);
            let maximum = u128::from(maximum_offset);
            let newer_distance = u128::from(top)
                .saturating_mul(maximum)
                .saturating_add(u128::from(travel) / 2)
                / u128::from(travel);
            maximum_offset.saturating_sub(
                u64::try_from(newer_distance.min(maximum)).unwrap_or(maximum_offset),
            )
        }
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
        previous_row: Option<u16>,
    }

    impl StatusRenderer {
        fn new(device: Option<String>, physical_size: TerminalSize) -> Self {
            Self {
                device,
                physical_size,
                path: TerminalViewConnectionPath::Unknown,
                rtt_ms: None,
                previous_row: None,
            }
        }

        fn enabled(&self) -> bool {
            self.device.is_some() && self.physical_size.rows > 1
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

        #[cfg(test)]
        fn render(
            &mut self,
            writer: &mut impl Write,
            transport_state: TerminalViewTransportState,
        ) -> Result<(), CliError> {
            present_atomic(writer, "render terminal status", |frame| {
                self.write(frame, transport_state)
            })
        }

        fn write(
            &mut self,
            writer: &mut impl Write,
            transport_state: TerminalViewTransportState,
        ) -> io::Result<()> {
            let current_row = self.enabled().then_some(self.physical_size.rows);
            let mut bytes = Vec::new();
            if let Some(previous) = self.previous_row
                && Some(previous) != current_row
                && previous <= self.physical_size.rows
            {
                bytes.extend_from_slice(b"\x1b7\x1b[");
                bytes.extend_from_slice(previous.to_string().as_bytes());
                bytes.extend_from_slice(b";1H\x1b[0m\x1b[K\x1b8");
            }
            if let (Some(row), Some(device)) = (current_row, self.device.as_deref()) {
                // Synchronizing is also used for an in-epoch visual snapshot
                // replacement. Only a genuine reconnect invalidates the
                // selected-path observation.
                let (path, latency) = if transport_state != TerminalViewTransportState::Reconnecting
                {
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
                let text = format!("{device} | {path} | {}", latency.as_deref().unwrap_or("--"));
                let (clipped, width) = clip_display_width(&text, self.physical_size.columns);
                bytes.extend_from_slice(b"\x1b7\x1b[");
                bytes.extend_from_slice(row.to_string().as_bytes());
                bytes.extend_from_slice(b";1H\x1b[0;7m");
                bytes.extend_from_slice(clipped.as_bytes());
                bytes.extend(std::iter::repeat_n(
                    b' ',
                    usize::from(self.physical_size.columns).saturating_sub(width),
                ));
                bytes.extend_from_slice(b"\x1b[K\x1b[0m\x1b8");
            }
            self.previous_row = current_row;
            writer.write_all(&bytes)
        }
    }

    fn clip_display_width(text: &str, maximum: u16) -> (String, usize) {
        let maximum = usize::from(maximum);
        let mut clipped = String::new();
        let mut width: usize = 0;
        for character in text.chars() {
            let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
            if width.saturating_add(character_width) > maximum {
                break;
            }
            clipped.push(character);
            width += character_width;
        }
        (clipped, width)
    }

    #[derive(Clone, Copy)]
    struct HistoryRequest {
        direction: TerminalHistoryDirection,
        cursor: Option<TerminalHistoryCursor>,
    }

    enum ViewportEffect {
        None,
        Render,
        Request(HistoryRequest),
        RequestViewport(TerminalScrollAction),
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

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum HistoryProtocol {
        Window,
        Viewport,
        Pager,
    }

    struct HistoryViewport {
        protocol: HistoryProtocol,
        page: Option<TerminalHistoryPage>,
        frame: Option<TerminalViewportFrame>,
        offset: usize,
        pending: Option<TerminalHistoryDirection>,
        viewport_pending: bool,
        queued_scroll: Option<TerminalScrollAction>,
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
        discard_viewport_response: bool,
        discard_window_response: bool,
        window_supported: bool,
        window_cache: ViewportCache<Vec<u8>>,
    }

    type VisibleHistoryRows<'a> = (&'a [Vec<u8>], usize, Option<&'static str>);

    impl ViewportController {
        #[cfg(test)]
        const fn new(content_size: TerminalSize) -> Self {
            Self {
                state: ViewportState::Live,
                content_size,
                live_metrics: None,
                gutter_column: None,
                drag_grab_row: None,
                drag_last_request: None,
                drag_deferred_target: None,
                last_presented_scroll_metrics: None,
                presented_gutter_column: None,
                discard_viewport_response: false,
                discard_window_response: false,
                window_supported: true,
                window_cache: ViewportCache::new(),
            }
        }

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
                discard_viewport_response: false,
                discard_window_response: false,
                window_supported: true,
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
            matches!(
                &self.state,
                ViewportState::History(history) if history.protocol == HistoryProtocol::Window
            ) && self.window_cache.visible_rows().is_some()
        }

        const fn is_resume_pending(&self) -> bool {
            matches!(&self.state, ViewportState::ResumePending { .. })
        }

        fn resize(&mut self, content_size: TerminalSize) {
            let changed = self.content_size != content_size;
            self.content_size = content_size;
            if let ViewportState::History(history) = &mut self.state
                && let Some(page) = &history.page
            {
                history.offset = history.offset.min(
                    page.rows
                        .len()
                        .saturating_sub(usize::from(content_size.rows)),
                );
            }
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
            if !self.window_supported
                || !self.is_live()
                || self
                    .live_metrics
                    .is_none_or(|metrics| metrics.max_offset_from_bottom == 0)
            {
                return None;
            }
            self.window_cache.set_target(0).request
        }

        fn refetch_history_window(&mut self) -> Option<TerminalHistoryWindowQuery> {
            let is_window_history = matches!(
                &self.state,
                ViewportState::History(history) if history.protocol == HistoryProtocol::Window
            );
            if !self.window_supported
                || !is_window_history
                || self.window_cache.visible_rows().is_some()
            {
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
                    if matches!(
                        &self.state,
                        ViewportState::History(history)
                            if history.protocol == HistoryProtocol::Window
                    ) && self.window_cache.desired_offset_from_bottom() == 0
                    {
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
                    if matches!(
                        &self.state,
                        ViewportState::History(history)
                            if history.protocol == HistoryProtocol::Window
                    ) && target == self.window_cache.desired_offset_from_bottom()
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
            if matches!(
                &self.state,
                ViewportState::History(history) if history.protocol == HistoryProtocol::Pager
            ) {
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
                if self.window_supported && self.window_cache.anchor().is_some() {
                    let maximum = self
                        .window_cache
                        .anchor()
                        .map_or(0, |anchor| anchor.max_offset_from_bottom);
                    let target = u64::try_from(amount).unwrap_or(u64::MAX).min(maximum);
                    let update = self.window_cache.set_target(target);
                    self.state = ViewportState::History(HistoryViewport {
                        protocol: HistoryProtocol::Window,
                        page: None,
                        frame: None,
                        offset: 0,
                        pending: None,
                        viewport_pending: false,
                        queued_scroll: None,
                        notice: None,
                    });
                    return history_window_effect(update);
                }
                if self.live_metrics.is_some() {
                    let action = scroll_lines_action(true, amount);
                    self.state = ViewportState::History(HistoryViewport {
                        protocol: HistoryProtocol::Viewport,
                        page: None,
                        frame: None,
                        offset: 0,
                        pending: None,
                        viewport_pending: true,
                        queued_scroll: None,
                        notice: Some(HISTORY_LOADING_NOTICE),
                    });
                    return ViewportEffect::RequestViewport(action);
                }
                self.state = ViewportState::History(HistoryViewport {
                    protocol: HistoryProtocol::Pager,
                    page: None,
                    frame: None,
                    offset: 0,
                    pending: Some(TerminalHistoryDirection::Newest),
                    viewport_pending: false,
                    queued_scroll: None,
                    notice: Some(HISTORY_LOADING_NOTICE),
                });
                return ViewportEffect::Request(HistoryRequest {
                    direction: TerminalHistoryDirection::Newest,
                    cursor: None,
                });
            }
            let ViewportState::History(history) = &mut self.state else {
                return ViewportEffect::None;
            };
            if history.protocol == HistoryProtocol::Window {
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
                return history_window_effect(self.window_cache.set_target(target));
            }
            if history.protocol == HistoryProtocol::Viewport {
                let action = scroll_lines_action(older, amount);
                if history.viewport_pending {
                    history.queued_scroll =
                        Some(coalesce_scroll_action(history.queued_scroll, action));
                    return ViewportEffect::None;
                }
                history.viewport_pending = true;
                return ViewportEffect::RequestViewport(action);
            }
            if history.pending.is_some() {
                return ViewportEffect::None;
            }
            let Some(page) = &history.page else {
                if older {
                    history.notice = Some(HISTORY_LOADING_NOTICE);
                    history.pending = Some(TerminalHistoryDirection::Newest);
                    return ViewportEffect::Request(HistoryRequest {
                        direction: TerminalHistoryDirection::Newest,
                        cursor: None,
                    });
                }
                return self.start_resume(Vec::new());
            };
            let maximum_offset = page
                .rows
                .len()
                .saturating_sub(usize::from(self.content_size.rows));
            if older {
                if history.offset > 0 {
                    history.offset = history.offset.saturating_sub(amount);
                    return ViewportEffect::Render;
                }
                if page.cursor.start_row > page.cursor.oldest_row {
                    history.pending = Some(TerminalHistoryDirection::Older);
                    return ViewportEffect::Request(HistoryRequest {
                        direction: TerminalHistoryDirection::Older,
                        cursor: Some(page.cursor),
                    });
                }
                ViewportEffect::None
            } else if history.offset < maximum_offset {
                history.offset = history.offset.saturating_add(amount).min(maximum_offset);
                ViewportEffect::Render
            } else if page
                .cursor
                .start_row
                .saturating_add(u64::from(page.cursor.row_count))
                < page.cursor.newest_row
            {
                history.pending = Some(TerminalHistoryDirection::Newer);
                ViewportEffect::Request(HistoryRequest {
                    direction: TerminalHistoryDirection::Newer,
                    cursor: Some(page.cursor),
                })
            } else {
                self.start_resume(Vec::new())
            }
        }

        fn apply_history(&mut self, result: TerminalHistoryResult) -> Result<(), CliError> {
            let ViewportState::History(history) = &mut self.state else {
                return Err(terminal_daemon_error(
                    DomainErrorKind::MalformedFrame,
                    "terminal history page arrived without a pending history view",
                ));
            };
            if history.protocol != HistoryProtocol::Pager {
                return Err(terminal_daemon_error(
                    DomainErrorKind::MalformedFrame,
                    "legacy history page arrived for a semantic viewport",
                ));
            }
            let direction = history.pending.take().ok_or_else(|| {
                terminal_daemon_error(
                    DomainErrorKind::MalformedFrame,
                    "terminal history page arrived without a pending request",
                )
            })?;
            match result {
                TerminalHistoryResult::Page(page) => {
                    history.offset = match direction {
                        TerminalHistoryDirection::Newest | TerminalHistoryDirection::Older => page
                            .rows
                            .len()
                            .saturating_sub(usize::from(self.content_size.rows)),
                        TerminalHistoryDirection::Newer => 0,
                    };
                    history.notice = page
                        .rows
                        .is_empty()
                        .then_some("[zterm: no retained history]");
                    history.page = Some(page);
                }
                TerminalHistoryResult::HistoryChanged { .. } => {
                    history.page = None;
                    history.offset = 0;
                    history.notice = Some(
                        "[zterm: retained history changed; press a normal key to return live]",
                    );
                }
                TerminalHistoryResult::HistoryGap { .. } => {
                    history.page = None;
                    history.offset = 0;
                    history.notice = Some(
                        "[zterm: retained history is no longer available; press a normal key to return live]",
                    );
                }
            }
            Ok(())
        }

        fn apply_viewport(
            &mut self,
            result: TerminalViewportResult,
        ) -> Result<ViewportEffect, CliError> {
            if self.discard_viewport_response {
                self.discard_viewport_response = false;
                return Ok(ViewportEffect::None);
            }
            let ViewportState::History(history) = &mut self.state else {
                return Err(terminal_daemon_error(
                    DomainErrorKind::MalformedFrame,
                    "terminal viewport frame arrived without a history view",
                ));
            };
            if history.protocol != HistoryProtocol::Viewport || !history.viewport_pending {
                return Err(terminal_daemon_error(
                    DomainErrorKind::MalformedFrame,
                    "terminal viewport frame arrived without a pending semantic request",
                ));
            }
            history.viewport_pending = false;
            match result {
                TerminalViewportResult::Frame(frame) => {
                    if let Some(action) = history.queued_scroll.take() {
                        // The server has advanced the attachment baseline, but
                        // this coalesced intermediate frame was never painted.
                        // Keep `history.frame` aligned with the last complete
                        // host presentation while requesting the final target.
                        history.viewport_pending = true;
                        return Ok(ViewportEffect::RequestViewport(action));
                    }
                    history.notice = (frame.disposition
                        == zterm_core::terminal::TerminalViewportDisposition::Rebased)
                        .then_some("[zterm: retained history changed; showing closest view]");
                    history.frame = Some(frame);
                    Ok(ViewportEffect::Render)
                }
                TerminalViewportResult::Live(metrics) => {
                    self.live_metrics = Some(metrics);
                    Ok(self.start_resume(Vec::new()))
                }
                TerminalViewportResult::HistoryChanged { .. } => {
                    history.queued_scroll = None;
                    Ok(ViewportEffect::None)
                }
                TerminalViewportResult::HistoryGap { epoch, revision }
                    if epoch.get() == 0 && revision.get() == 0 =>
                {
                    history.protocol = HistoryProtocol::Pager;
                    history.page = None;
                    history.frame = None;
                    history.offset = 0;
                    history.pending = Some(TerminalHistoryDirection::Newest);
                    history.viewport_pending = false;
                    history.queued_scroll = None;
                    history.notice = Some(HISTORY_LOADING_NOTICE);
                    Ok(ViewportEffect::Request(HistoryRequest {
                        direction: TerminalHistoryDirection::Newest,
                        cursor: None,
                    }))
                }
                TerminalViewportResult::HistoryGap { .. } => {
                    history.queued_scroll = None;
                    Ok(ViewportEffect::None)
                }
            }
        }

        fn apply_history_window(
            &mut self,
            result: TerminalHistoryWindowResult,
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
                TerminalHistoryWindowResult::Frame(frame) => {
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
                            rows: frame.ansi_rows,
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
                        if let ViewportState::History(history) = &mut self.state
                            && history.protocol == HistoryProtocol::Window
                        {
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
                TerminalHistoryWindowResult::HistoryChanged { epoch, revision }
                | TerminalHistoryWindowResult::HistoryGap { epoch, revision } => {
                    self.window_cache.defer_pending_request();
                    self.window_cache.invalidate_rows();
                    if epoch.get() == 0 && revision.get() == 0 {
                        self.window_supported = false;
                        let target = self.window_cache.desired_offset_from_bottom();
                        if target == 0 {
                            return Ok(ViewportEffect::None);
                        }
                        self.state = ViewportState::History(HistoryViewport {
                            protocol: HistoryProtocol::Viewport,
                            page: None,
                            frame: None,
                            offset: 0,
                            pending: None,
                            viewport_pending: true,
                            queued_scroll: None,
                            notice: None,
                        });
                        return Ok(ViewportEffect::RequestViewport(
                            TerminalScrollAction::ScrollToOffset(target),
                        ));
                    }
                    // A content-free changed/gap response is not a complete
                    // replacement. Keep the last committed host frame intact;
                    // the next gesture may retry or normal input may resume.
                    Ok(ViewportEffect::None)
                }
            }
        }

        fn scroll_to_offset(&mut self, offset: u64) -> ViewportEffect {
            let ViewportState::History(history) = &mut self.state else {
                if self.live_metrics.is_none() || offset == 0 {
                    return ViewportEffect::None;
                }
                if self.window_supported && self.window_cache.anchor().is_some() {
                    let update = self.window_cache.set_target(offset);
                    self.state = ViewportState::History(HistoryViewport {
                        protocol: HistoryProtocol::Window,
                        page: None,
                        frame: None,
                        offset: 0,
                        pending: None,
                        viewport_pending: false,
                        queued_scroll: None,
                        notice: None,
                    });
                    return history_window_effect(update);
                }
                self.state = ViewportState::History(HistoryViewport {
                    protocol: HistoryProtocol::Viewport,
                    page: None,
                    frame: None,
                    offset: 0,
                    pending: None,
                    viewport_pending: true,
                    queued_scroll: None,
                    notice: Some(HISTORY_LOADING_NOTICE),
                });
                return ViewportEffect::RequestViewport(TerminalScrollAction::ScrollToOffset(
                    offset,
                ));
            };
            if history.protocol == HistoryProtocol::Window {
                if offset == 0 {
                    if self.drag_grab_row.is_some() {
                        return history_window_effect(self.window_cache.set_target(0));
                    }
                    return self.start_resume(Vec::new());
                }
                return history_window_effect(self.window_cache.set_target(offset));
            }
            if history.protocol != HistoryProtocol::Viewport {
                return ViewportEffect::None;
            }
            let action = TerminalScrollAction::ScrollToOffset(offset);
            if history.viewport_pending {
                history.queued_scroll = Some(action);
                ViewportEffect::None
            } else {
                history.viewport_pending = true;
                ViewportEffect::RequestViewport(action)
            }
        }

        fn scroll_metrics(&self) -> Option<TerminalScrollMetrics> {
            match &self.state {
                ViewportState::Live => self.live_metrics,
                ViewportState::History(history)
                    if history.protocol == HistoryProtocol::Viewport =>
                {
                    history
                        .frame
                        .as_ref()
                        .map(|frame| frame.metrics)
                        .or(self.live_metrics)
                }
                ViewportState::History(history) if history.protocol == HistoryProtocol::Window => {
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
                ViewportState::History(_) => None,
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
            self.last_presented_scroll_metrics = self.scroll_metrics();
            self.presented_gutter_column = self.gutter_column;
        }

        fn observe_live_metrics(&mut self, metrics: Option<TerminalScrollMetrics>) {
            if metrics.is_some() || self.is_live() || self.is_resume_pending() {
                self.live_metrics = metrics;
            }
            self.observe_window_anchor(metrics);
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
            self.discard_viewport_response |= matches!(
                &self.state,
                ViewportState::History(history)
                    if history.protocol == HistoryProtocol::Viewport && history.viewport_pending
            );
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

        fn visible_history_rows(&self) -> Option<VisibleHistoryRows<'_>> {
            let ViewportState::History(history) = &self.state else {
                return None;
            };
            if history.protocol == HistoryProtocol::Window {
                let rows = self.window_cache.visible_rows()?;
                return Some((rows, usize::from(self.content_size.rows), history.notice));
            }
            if history.protocol == HistoryProtocol::Viewport {
                let frame = history.frame.as_ref()?;
                let end = usize::from(self.content_size.rows).min(frame.rows.len());
                let rows = &frame.rows[..end];
                return Some((rows, usize::from(self.content_size.rows), history.notice));
            }
            let page = history.page.as_ref()?;
            let end = history
                .offset
                .saturating_add(usize::from(self.content_size.rows))
                .min(page.rows.len());
            let rows = &page.rows[history.offset.min(end)..end];
            Some((rows, usize::from(self.content_size.rows), history.notice))
        }
    }

    fn scroll_lines_action(older: bool, amount: usize) -> TerminalScrollAction {
        let amount = i32::try_from(amount).unwrap_or(i32::MAX);
        TerminalScrollAction::ScrollByLines(if older {
            amount
        } else {
            amount.saturating_neg()
        })
    }

    fn history_window_effect(update: ViewportCacheUpdate) -> ViewportEffect {
        match (update.render_local, update.request) {
            (true, Some(query)) => ViewportEffect::RenderAndRequestHistoryWindow(query),
            (true, None) => ViewportEffect::Render,
            (false, Some(query)) => ViewportEffect::RequestHistoryWindow(query),
            (false, None) => ViewportEffect::None,
        }
    }

    fn coalesce_scroll_action(
        queued: Option<TerminalScrollAction>,
        latest: TerminalScrollAction,
    ) -> TerminalScrollAction {
        match (queued, latest) {
            (
                Some(TerminalScrollAction::ScrollByLines(previous)),
                TerminalScrollAction::ScrollByLines(latest),
            ) => TerminalScrollAction::ScrollByLines(previous.saturating_add(latest)),
            (_, latest) => latest,
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

    #[derive(Clone, Eq, PartialEq)]
    enum HostInputEvent {
        Bytes(Vec<u8>),
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
                            push_host_bytes(&mut events, raw);
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

    #[derive(Clone, Eq, PartialEq)]
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

    fn present_atomic(
        writer: &mut impl Write,
        operation: &'static str,
        build: impl FnOnce(&mut Vec<u8>) -> io::Result<()>,
    ) -> Result<(), CliError> {
        let mut frame = Vec::new();
        frame.extend_from_slice(HOST_SYNC_BEGIN);
        build(&mut frame).map_err(|error| terminal_io(operation, error))?;
        frame.extend_from_slice(HOST_INPUT_CAPTURE);
        frame.extend_from_slice(HOST_SYNC_END);
        if let Err(error) = writer.write_all(&frame) {
            // `write_all` may have emitted the synchronized-update opener
            // before failing. Always make a best-effort attempt to release the
            // outer terminal before returning the original I/O error.
            let _ = writer.write_all(HOST_SYNC_END);
            let _ = writer.flush();
            return Err(terminal_io(operation, error));
        }
        writer
            .flush()
            .map_err(|error| terminal_io(operation, error))
    }

    struct TerminalRenderer {
        revision: Option<Revision>,
        active_screen: ActiveScreen,
        modes: TerminalModes,
    }

    impl TerminalRenderer {
        const fn new() -> Self {
            Self {
                revision: None,
                active_screen: ActiveScreen::Main,
                modes: TerminalModes {
                    application_keypad: false,
                    application_cursor: false,
                    bracketed_paste: false,
                    focus_reporting: false,
                    alternate_scroll: false,
                    mouse_mode: TerminalMouseMode::None,
                    mouse_encoding: TerminalMouseEncoding::Default,
                },
            }
        }

        #[cfg(test)]
        fn apply_snapshot(
            &mut self,
            writer: &mut impl Write,
            snapshot: RenderSnapshot<'_>,
        ) -> Result<(), CliError> {
            self.apply_snapshot_with_chrome(writer, snapshot, |_| Ok(()))
        }

        fn apply_snapshot_with_chrome<Writer, Chrome>(
            &mut self,
            writer: &mut Writer,
            snapshot: RenderSnapshot<'_>,
            chrome: Chrome,
        ) -> Result<(), CliError>
        where
            Writer: Write,
            Chrome: FnOnce(&mut Vec<u8>) -> io::Result<()>,
        {
            let screen = snapshot_screen_ansi(snapshot.screen_ansi, snapshot.active_screen)
                .map_err(|error| terminal_io("render terminal snapshot", error))?;
            present_atomic(writer, "render terminal snapshot", |frame| {
                frame.write_all(snapshot.recent_history_ansi)?;
                frame.write_all(screen)?;
                chrome(frame)
            })?;
            self.revision = Some(snapshot.revision);
            self.active_screen = snapshot.active_screen;
            self.modes = snapshot.modes;
            Ok(())
        }

        #[cfg(test)]
        fn apply_delta(
            &mut self,
            writer: &mut impl Write,
            delta: RenderDelta<'_>,
        ) -> Result<DeltaRender, CliError> {
            self.apply_delta_with_chrome(writer, delta, |_| Ok(()))
        }

        fn apply_delta_with_chrome<Writer, Chrome>(
            &mut self,
            writer: &mut Writer,
            delta: RenderDelta<'_>,
            chrome: Chrome,
        ) -> Result<DeltaRender, CliError>
        where
            Writer: Write,
            Chrome: FnOnce(&mut Vec<u8>) -> io::Result<()>,
        {
            let Some(ansi) = self.validate_delta(delta)? else {
                return Ok(DeltaRender::Gap);
            };
            present_atomic(writer, "render terminal delta", |frame| {
                frame.write_all(ansi)?;
                chrome(frame)
            })?;
            self.revision = Some(delta.to_revision);
            self.active_screen = delta.active_screen;
            self.modes = delta.modes;
            Ok(DeltaRender::Applied)
        }

        fn observe_snapshot(&mut self, snapshot: RenderSnapshot<'_>) -> Result<(), CliError> {
            snapshot_screen_ansi(snapshot.screen_ansi, snapshot.active_screen)
                .map_err(|error| terminal_io("validate terminal snapshot", error))?;
            self.revision = Some(snapshot.revision);
            self.active_screen = snapshot.active_screen;
            self.modes = snapshot.modes;
            Ok(())
        }

        fn observe_delta(&mut self, delta: RenderDelta<'_>) -> Result<DeltaRender, CliError> {
            if self.validate_delta(delta)?.is_none() {
                return Ok(DeltaRender::Gap);
            }
            self.revision = Some(delta.to_revision);
            self.active_screen = delta.active_screen;
            self.modes = delta.modes;
            Ok(DeltaRender::Applied)
        }

        fn validate_delta<'a>(&self, delta: RenderDelta<'a>) -> Result<Option<&'a [u8]>, CliError> {
            if self.revision != Some(delta.from_revision) {
                return Ok(None);
            }
            if delta.to_revision.get() <= delta.from_revision.get() {
                return Err(terminal_daemon_error(
                    DomainErrorKind::MalformedFrame,
                    "terminal delta revision did not advance",
                ));
            }
            delta_screen_ansi(delta.ansi, self.active_screen, delta.active_screen)
                .map(Some)
                .map_err(|error| terminal_io("validate terminal delta", error))
        }

        fn revision(&self) -> Revision {
            self.revision
                .expect("the initial snapshot is rendered before event processing")
        }

        const fn active_screen(&self) -> ActiveScreen {
            self.active_screen
        }

        const fn modes(&self) -> TerminalModes {
            self.modes
        }
    }

    struct RenderSnapshot<'a> {
        revision: Revision,
        active_screen: ActiveScreen,
        modes: TerminalModes,
        recent_history_ansi: &'a [u8],
        screen_ansi: &'a [u8],
    }

    impl<'a> From<&'a TerminalViewSnapshot> for RenderSnapshot<'a> {
        fn from(snapshot: &'a TerminalViewSnapshot) -> Self {
            Self {
                revision: snapshot.revision(),
                active_screen: snapshot.active_screen(),
                modes: snapshot.modes(),
                recent_history_ansi: snapshot.recent_history_ansi(),
                screen_ansi: snapshot.screen_ansi(),
            }
        }
    }

    #[derive(Clone, Copy)]
    struct RenderDelta<'a> {
        from_revision: Revision,
        to_revision: Revision,
        active_screen: ActiveScreen,
        modes: TerminalModes,
        ansi: &'a [u8],
    }

    impl<'a> From<&'a TerminalViewDelta> for RenderDelta<'a> {
        fn from(delta: &'a TerminalViewDelta) -> Self {
            Self {
                from_revision: delta.from_revision(),
                to_revision: delta.to_revision(),
                active_screen: delta.active_screen(),
                modes: delta.modes(),
                ansi: delta.ansi(),
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum DeltaRender {
        Applied,
        Gap,
    }

    fn render_snapshot_stdout(
        renderer: &mut TerminalRenderer,
        snapshot: &TerminalViewSnapshot,
        viewport: &ViewportController,
        status: &mut StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<(), CliError> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        renderer.apply_snapshot_with_chrome(&mut output, snapshot.into(), |writer| {
            write_chrome(writer, viewport, status, transport_state)
        })
    }

    fn render_delta_stdout(
        renderer: &mut TerminalRenderer,
        delta: &TerminalViewDelta,
        viewport: &ViewportController,
        status: &mut StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<DeltaRender, CliError> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        renderer.apply_delta_with_chrome(&mut output, delta.into(), |writer| {
            write_chrome(writer, viewport, status, transport_state)
        })
    }

    fn render_transport_state_stdout(state: TerminalViewTransportState) -> Result<(), CliError> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        render_transport_state(&mut output, state)
    }

    fn render_chrome_with_writer(
        writer: &mut impl Write,
        viewport: &ViewportController,
        renderer: &mut StatusRenderer,
        state: TerminalViewTransportState,
    ) -> Result<(), CliError> {
        present_atomic(writer, "render terminal chrome", |frame| {
            write_chrome(frame, viewport, renderer, state)
        })
    }

    fn render_history_stdout(
        viewport: &ViewportController,
        status: &mut StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<(), CliError> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        render_history_with_chrome(&mut output, viewport, status, transport_state)
    }

    fn present_cached_viewport_with_writer(
        writer: &mut impl Write,
        viewport: &mut ViewportController,
        status: &mut StatusRenderer,
        transport_state: TerminalViewTransportState,
        pacer: &mut ViewportPresentationPacer,
        now: Instant,
        force: bool,
    ) -> Result<bool, CliError> {
        if !force && !pacer.due(now) {
            return Ok(false);
        }
        if !viewport.has_complete_cached_viewport() {
            pacer.cancel();
            return Ok(false);
        }
        render_history_with_chrome(writer, viewport, status, transport_state)?;
        viewport.observe_presentation();
        pacer.mark_presented(now);
        Ok(true)
    }

    fn mark_cached_viewport_dirty(
        viewport: &ViewportController,
        pacer: &mut ViewportPresentationPacer,
        now: Instant,
    ) -> bool {
        if !viewport.has_complete_cached_viewport() {
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
        if !viewport.has_complete_cached_viewport() {
            pacer.cancel();
        }
    }

    fn present_cached_viewport_stdout(
        viewport: &mut ViewportController,
        status: &mut StatusRenderer,
        transport_state: TerminalViewTransportState,
        pacer: &mut ViewportPresentationPacer,
        now: Instant,
        force: bool,
    ) -> Result<bool, CliError> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        present_cached_viewport_with_writer(
            &mut output,
            viewport,
            status,
            transport_state,
            pacer,
            now,
            force,
        )
    }

    fn write_scrollbar(writer: &mut impl Write, viewport: &ViewportController) -> io::Result<()> {
        let presented_column = viewport.presented_gutter_column;
        if let (Some(column), Some(current_column)) = (presented_column, viewport.gutter_column)
            && column != current_column
        {
            writer.write_all(b"\x1b7\x1b[0m")?;
            for row in 0..viewport.content_size.rows {
                write!(writer, "\x1b[{};{}H ", row + 1, column)?;
            }
            writer.write_all(b"\x1b[0m\x1b8")?;
        }
        let Some(column) = viewport.gutter_column else {
            // Alternate screen snapshots repaint the reclaimed column before
            // chrome is composed, while a <=4-column physical resize clips the
            // old gutter. Clearing either afterward could overwrite child
            // content through a right-margin clamp.
            return Ok(());
        };
        let geometry = viewport
            .scroll_metrics()
            .and_then(|metrics| ScrollbarGeometry::new(viewport.content_size.rows, metrics));
        writer.write_all(b"\x1b7\x1b[0m")?;
        for row in 0..viewport.content_size.rows {
            write!(writer, "\x1b[{};{}H", row + 1, column)?;
            let glyph = match geometry {
                Some(geometry)
                    if row >= geometry.thumb_top
                        && row < geometry.thumb_top.saturating_add(geometry.thumb_len) =>
                {
                    "▐"
                }
                Some(_) => "▕",
                None => " ",
            };
            writer.write_all(glyph.as_bytes())?;
        }
        writer.write_all(b"\x1b[0m\x1b8")?;
        Ok(())
    }

    fn write_chrome(
        writer: &mut impl Write,
        viewport: &ViewportController,
        status: &mut StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> io::Result<()> {
        status.write(writer, transport_state)?;
        write_scrollbar(writer, viewport)
    }

    #[cfg(test)]
    fn render_scrollbar(
        writer: &mut impl Write,
        viewport: &mut ViewportController,
    ) -> Result<(), CliError> {
        present_atomic(writer, "render terminal scrollbar", |frame| {
            write_scrollbar(frame, viewport)
        })?;
        viewport.observe_presentation();
        Ok(())
    }

    fn render_history_with_chrome(
        writer: &mut impl Write,
        viewport: &ViewportController,
        status: &mut StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<(), CliError> {
        present_atomic(writer, "render terminal history and chrome", |frame| {
            write_history(frame, viewport)?;
            write_chrome(frame, viewport, status, transport_state)
        })
    }

    fn write_history(writer: &mut impl Write, viewport: &ViewportController) -> io::Result<()> {
        let Some((rows, height, notice)) = viewport.visible_history_rows() else {
            // Request, resize, resume, and content-free gap states keep the
            // last complete host presentation untouched until its replacement
            // is ready. Chrome may still be repaired in the same transaction.
            return Ok(());
        };
        let top_padding = height.saturating_sub(rows.len());
        writer.write_all(b"\x1b[?25l\x1b[0m")?;
        for terminal_row in 0..height {
            write!(writer, "\x1b[{};1H\x1b[0m", terminal_row + 1)?;
            if terminal_row == 0
                && let Some(notice) = notice
            {
                writer.write_all(notice.as_bytes())?;
            } else if terminal_row >= top_padding {
                let index = terminal_row - top_padding;
                if let Some(row) = rows.get(index) {
                    writer.write_all(row)?;
                }
            }
            writer.write_all(b"\x1b[K")?;
        }
        Ok(())
    }

    fn render_view_stdout(
        viewport: &ViewportController,
        status: &mut StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<(), CliError> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        render_view_with_writer(&mut output, viewport, status, transport_state)
    }

    fn render_view_with_writer(
        writer: &mut impl Write,
        viewport: &ViewportController,
        status: &mut StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<(), CliError> {
        if viewport.is_history() || viewport.is_resume_pending() {
            render_history_with_chrome(writer, viewport, status, transport_state)
        } else {
            render_chrome_with_writer(writer, viewport, status, transport_state)
        }
    }

    fn render_transport_transition_view_with_writer(
        writer: &mut impl Write,
        viewport: &ViewportController,
        status: &mut StatusRenderer,
        transport_state: TerminalViewTransportState,
        resumed_from_snapshot: bool,
    ) -> Result<bool, CliError> {
        let resume_sync_is_visually_unchanged = viewport.is_resume_pending()
            && transport_state == TerminalViewTransportState::Synchronizing;
        // The old complete history frame remains authoritative until the
        // snapshot replaces it. Once that snapshot has been painted, changing
        // ResumePending to Live changes no pixels either.
        if resumed_from_snapshot || resume_sync_is_visually_unchanged {
            return Ok(false);
        }
        render_view_with_writer(writer, viewport, status, transport_state)?;
        Ok(true)
    }

    fn render_transport_transition_view_stdout(
        viewport: &ViewportController,
        status: &mut StatusRenderer,
        transport_state: TerminalViewTransportState,
        resumed_from_snapshot: bool,
    ) -> Result<bool, CliError> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        render_transport_transition_view_with_writer(
            &mut output,
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
        writer: &zterm_daemon::operations::TerminalViewCommandWriter,
        revision: Revision,
        status: &mut StatusRenderer,
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
                            viewport,
                            status,
                            transport_state,
                            pacer,
                            now,
                            false,
                        )?;
                    }
                } else {
                    pacer.cancel();
                    render_view_stdout(viewport, status, transport_state)?;
                    viewport.observe_presentation();
                    pacer.mark_presented(now);
                }
                Ok(false)
            }
            ViewportEffect::Request(request) => {
                pacer.cancel();
                writer
                    .request_history(request.direction, request.cursor, MAX_HISTORY_PAGE_ROWS)
                    .await?;
                Ok(false)
            }
            ViewportEffect::RequestViewport(action) => {
                pacer.cancel();
                writer.request_viewport(action).await?;
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
                if mark_cached_viewport_dirty(viewport, pacer, now) {
                    if !defer_cached_presentation {
                        let _ = present_cached_viewport_stdout(
                            viewport,
                            status,
                            transport_state,
                            pacer,
                            now,
                            false,
                        )?;
                    }
                } else {
                    pacer.cancel();
                }
                Ok(false)
            }
            ViewportEffect::Resume => {
                pacer.cancel();
                writer.request_sync(revision).await?;
                Ok(true)
            }
        }
    }

    fn render_transport_state(
        writer: &mut impl Write,
        state: TerminalViewTransportState,
    ) -> Result<(), CliError> {
        if state != TerminalViewTransportState::Reconnecting {
            return Ok(());
        }
        present_atomic(writer, "render terminal transport state", |frame| {
            frame.write_all(RECONNECTING_STATUS)
        })
    }

    fn snapshot_screen_ansi(ansi: &[u8], active_screen: ActiveScreen) -> io::Result<&[u8]> {
        let ansi = ansi
            .strip_prefix(MAIN_SCREEN_SELECTION_ANSI)
            .ok_or_else(|| io::Error::other("snapshot omitted zterm's main-screen selector"))?;
        let ansi = match active_screen {
            ActiveScreen::Main => Ok(ansi),
            ActiveScreen::Alternate => ansi
                .strip_prefix(ALTERNATE_SCREEN_SELECTION_ANSI)
                .ok_or_else(|| {
                    io::Error::other("alternate snapshot omitted zterm's screen selector")
                }),
        }?;
        reject_nested_screen_selection(ansi)?;
        Ok(ansi)
    }

    fn delta_screen_ansi(
        ansi: &[u8],
        previous_screen: ActiveScreen,
        active_screen: ActiveScreen,
    ) -> io::Result<&[u8]> {
        let (selected_screen, ansi) =
            if let Some(ansi) = ansi.strip_prefix(MAIN_SCREEN_SELECTION_ANSI) {
                (Some(ActiveScreen::Main), ansi)
            } else if let Some(ansi) = ansi.strip_prefix(ALTERNATE_SCREEN_SELECTION_ANSI) {
                (Some(ActiveScreen::Alternate), ansi)
            } else {
                (None, ansi)
            };
        if selected_screen.is_some_and(|selected| selected != active_screen) {
            return Err(io::Error::other(
                "terminal delta selected a screen inconsistent with its metadata",
            ));
        }
        if previous_screen != active_screen && selected_screen != Some(active_screen) {
            return Err(io::Error::other(
                "terminal delta changed screens without zterm's screen selector",
            ));
        }
        reject_nested_screen_selection(ansi)?;
        Ok(ansi)
    }

    fn reject_nested_screen_selection(ansi: &[u8]) -> io::Result<()> {
        if contains_bytes(ansi, MAIN_SCREEN_SELECTION_ANSI)
            || contains_bytes(ansi, ALTERNATE_SCREEN_SELECTION_ANSI)
        {
            return Err(io::Error::other(
                "terminal ANSI contained a nested screen selector",
            ));
        }
        Ok(())
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
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

        fn cached_window_viewport() -> (ViewportController, TerminalSize) {
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
                panic!("initial cached-window fixture must request history");
            };
            let shape = query
                .response_shape(query.anchor)
                .expect("fixture query has a valid response shape");
            let rows = (0..shape.row_count)
                .map(|index| {
                    shape
                        .first_row_from_live_top
                        .saturating_add(i64::try_from(index).expect("fixture row fits i64"))
                        .to_string()
                        .into_bytes()
                })
                .collect();
            assert!(matches!(
                viewport
                    .apply_history_window(TerminalHistoryWindowResult::Frame(
                        zterm_core::terminal::TerminalHistoryWindowFrame {
                            disposition: shape.disposition,
                            anchor: query.anchor,
                            target_offset_from_bottom: shape.target_offset_from_bottom,
                            first_row_from_live_top: shape.first_row_from_live_top,
                            ansi_rows: rows,
                        },
                    ))
                    .expect("install fixture history window"),
                ViewportEffect::Render
            ));
            let mut status = StatusRenderer::new(None, physical);
            render_history_with_chrome(
                &mut Vec::new(),
                &viewport,
                &mut status,
                TerminalViewTransportState::Active,
            )
            .expect("present fixture history window");
            viewport.observe_presentation();
            (viewport, physical)
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
                    discard_viewport_response: false,
                    discard_window_response: false,
                    window_supported: true,
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

        #[test]
        fn resize_coalescer_retains_only_the_latest_non_active_viewport() {
            let initial = TerminalSize::new(24, 80);
            let first = TerminalSize::new(20, 60);
            let latest = TerminalSize::new(40, 120);
            let active = TerminalSize::new(50, 140);
            let mut coalescer = ResizeCoalescer::new(initial);
            assert_eq!(
                coalescer.observe(first, TerminalViewTransportState::Synchronizing),
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
                coalescer.observe(latest, TerminalViewTransportState::Synchronizing),
                None
            );
            assert_eq!(
                coalescer.enter_transport_state(TerminalViewTransportState::Active),
                (TerminalViewTransportState::Active, None)
            );
            assert_eq!(
                coalescer.observe(active, TerminalViewTransportState::Active),
                Some(active)
            );
        }

        #[test]
        fn remote_geometry_reserves_one_status_row_with_a_one_row_fallback() {
            let ordinary = TerminalSize::new(24, 80);
            assert_eq!(
                child_terminal_size(ordinary, false),
                TerminalSize::new(24, 79)
            );
            assert_eq!(
                child_terminal_size(ordinary, true),
                TerminalSize::new(23, 79)
            );
            assert_eq!(
                child_terminal_size(TerminalSize::new(2, 37), true),
                TerminalSize::new(1, 36)
            );
            assert_eq!(
                child_terminal_size(TerminalSize::new(1, 37), true),
                TerminalSize::new(1, 36)
            );
        }

        #[test]
        fn oversized_physical_geometry_projects_one_valid_initial_and_resize_viewport() {
            let oversized = TerminalSize::new(100, 300);
            let maximum = TerminalSize::new(80, 239);
            assert_eq!(child_terminal_size(oversized, false), maximum);
            assert_eq!(child_terminal_size(oversized, true), maximum);

            let mut coalescer = ResizeCoalescer::new(TerminalSize::new(24, 80));
            assert_eq!(
                coalescer.observe(
                    child_terminal_size(oversized, true),
                    TerminalViewTransportState::Active,
                ),
                Some(maximum)
            );
        }

        #[test]
        fn status_row_is_reverse_video_full_width_exact_and_unicode_clipped() {
            let mut renderer =
                StatusRenderer::new(Some("开发机".to_owned()), TerminalSize::new(4, 26));
            renderer.path = TerminalViewConnectionPath::Direct;
            renderer.rtt_ms = Some(42);
            let mut output = Vec::new();
            renderer
                .render(&mut output, TerminalViewTransportState::Active)
                .expect("render active direct status");
            assert_eq!(
                output,
                [
                    HOST_SYNC_BEGIN,
                    b"\x1b7\x1b[4;1H\x1b[0;7m\xe5\xbc\x80\xe5\x8f\x91\xe6\x9c\xba | direct | 42 ms   \x1b[K\x1b[0m\x1b8".as_slice(),
                    HOST_INPUT_CAPTURE,
                    HOST_SYNC_END,
                ]
                .concat()
            );

            let mut inactive =
                StatusRenderer::new(Some("开发机".to_owned()), TerminalSize::new(3, 26));
            inactive.path = TerminalViewConnectionPath::Relay;
            inactive.rtt_ms = Some(7);
            let mut inactive_output = Vec::new();
            inactive
                .render(
                    &mut inactive_output,
                    TerminalViewTransportState::Reconnecting,
                )
                .expect("render inactive status");
            assert!(contains_bytes(
                &inactive_output,
                "开发机 | -- | --".as_bytes()
            ));
            assert!(!contains_bytes(&inactive_output, b"relay"));
            assert!(!contains_bytes(&inactive_output, b"7 ms"));

            let mut same_epoch =
                StatusRenderer::new(Some("开发机".to_owned()), TerminalSize::new(3, 26));
            same_epoch.path = TerminalViewConnectionPath::Relay;
            same_epoch.rtt_ms = Some(7);
            let mut same_epoch_output = Vec::new();
            same_epoch
                .render(
                    &mut same_epoch_output,
                    TerminalViewTransportState::Synchronizing,
                )
                .expect("render same-epoch synchronizing status");
            assert!(contains_bytes(
                &same_epoch_output,
                "开发机 | relay | 7 ms".as_bytes()
            ));
            assert!(!contains_bytes(
                &same_epoch_output,
                "开发机 | -- | --".as_bytes()
            ));

            let mut narrow =
                StatusRenderer::new(Some("开发机".to_owned()), TerminalSize::new(2, 5));
            narrow.path = TerminalViewConnectionPath::Direct;
            narrow.rtt_ms = Some(42);
            let mut narrow_output = Vec::new();
            narrow
                .render(&mut narrow_output, TerminalViewTransportState::Active)
                .expect("render narrow Unicode status");
            assert_eq!(
                narrow_output,
                [
                    HOST_SYNC_BEGIN,
                    b"\x1b7\x1b[2;1H\x1b[0;7m\xe5\xbc\x80\xe5\x8f\x91 \x1b[K\x1b[0m\x1b8"
                        .as_slice(),
                    HOST_INPUT_CAPTURE,
                    HOST_SYNC_END,
                ]
                .concat()
            );
        }

        #[test]
        fn true_reconnect_clears_the_old_path_through_replacement_snapshot() {
            let physical = TerminalSize::new(4, 30);
            let layout = ChromeLayout::new(physical, true, ActiveScreen::Main);
            let mut viewport = ViewportController::with_layout(layout, None);
            let mut status = StatusRenderer::new(Some("remote".to_owned()), physical);
            status.path = TerminalViewConnectionPath::Direct;
            status.rtt_ms = Some(8);
            let mut output = ViewportFrameWriter::default();

            render_chrome_with_writer(
                &mut output,
                &viewport,
                &mut status,
                TerminalViewTransportState::Active,
            )
            .expect("present the old active path");
            viewport.observe_presentation();
            status.reset_for_reconnect();
            viewport.reset_presentation_for_reconnect();
            assert!(
                render_transport_transition_view_with_writer(
                    &mut output,
                    &viewport,
                    &mut status,
                    TerminalViewTransportState::Reconnecting,
                    false,
                )
                .expect("present the true reconnect state")
            );
            viewport.observe_presentation();
            let mut renderer = TerminalRenderer::new();
            renderer
                .apply_snapshot_with_chrome(
                    &mut output,
                    RenderSnapshot {
                        revision: Revision::new(3),
                        active_screen: ActiveScreen::Main,
                        modes: TerminalModes::default(),
                        recent_history_ansi: b"",
                        screen_ansi: b"\x1b[?1049lreplacement-live",
                    },
                    |writer| {
                        write_chrome(
                            writer,
                            &viewport,
                            &mut status,
                            TerminalViewTransportState::Synchronizing,
                        )
                    },
                )
                .expect("present the replacement-stream snapshot");
            viewport.observe_presentation();

            assert_eq!(output.frames.len(), 3);
            assert!(contains_bytes(&output.frames[0], b"remote | direct | 8 ms"));
            for frame in &output.frames[1..] {
                assert!(contains_bytes(frame, b"remote | -- | --"));
                assert!(!contains_bytes(frame, b"direct"));
                assert!(!contains_bytes(frame, b"8 ms"));
            }
            assert!(contains_bytes(&output.frames[2], b"replacement-live"));
            for frame in &output.frames {
                assert!(frame.starts_with(HOST_SYNC_BEGIN));
                assert!(contains_bytes(frame, HOST_INPUT_CAPTURE));
                assert!(frame.ends_with(HOST_SYNC_END));
            }
            assert_eq!(output.writes, 3);
            assert_eq!(output.flushes, 3);
        }

        #[test]
        fn host_codec_and_modes_route_page_and_wheel_without_program_branches() {
            let mut codec = HostInputCodec::new();
            assert!(
                codec
                    .feed(b"\x1b[5")
                    .expect("partial page sequence remains bounded")
                    .is_empty()
            );
            let page_events = codec
                .feed(b"~\x1b[6~raw")
                .expect("page and raw input decode");
            assert!(matches!(page_events.first(), Some(HostInputEvent::PageUp)));
            assert!(matches!(page_events.get(1), Some(HostInputEvent::PageDown)));
            assert!(matches!(
                page_events.get(2),
                Some(HostInputEvent::Bytes(bytes)) if bytes == b"raw"
            ));

            let wheel_raw = b"\x1b[<64;5;4M";
            let wheel_events = codec.feed(wheel_raw).expect("SGR mouse input decodes");
            let Some(HostInputEvent::Mouse(wheel)) = wheel_events.first() else {
                panic!("SGR wheel input must decode as one host mouse event");
            };
            assert!(wheel.is_wheel());
            assert!(wheel.wheel_is_up());

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

            assert!(!history_owns_gestures(ActiveScreen::Alternate, shell_modes));
            assert_eq!(
                route_mouse_to_child(wheel, ActiveScreen::Alternate, shell_modes),
                None
            );

            let alternate_scroll = TerminalModes {
                alternate_scroll: true,
                application_cursor: true,
                ..TerminalModes::default()
            };
            assert!(history_owns_gestures(ActiveScreen::Main, alternate_scroll));
            assert_eq!(
                route_mouse_to_child(wheel, ActiveScreen::Main, alternate_scroll),
                None
            );
            assert_eq!(
                route_mouse_to_child(wheel, ActiveScreen::Alternate, alternate_scroll),
                Some(b"\x1bOA".to_vec())
            );

            let child_burst = codec
                .feed(b"\x1b[<64;5;4M\x1b[<64;5;4M\x1b[<64;5;4M")
                .expect("three child-owned reports decode independently");
            let forwarded = child_burst
                .iter()
                .map(|event| {
                    let HostInputEvent::Mouse(mouse) = event else {
                        panic!("child burst contains only mouse reports");
                    };
                    route_mouse_to_child(mouse, ActiveScreen::Main, child_mouse)
                        .expect("child mouse mode owns every report")
                })
                .collect::<Vec<_>>();
            assert_eq!(forwarded, vec![wheel_raw.to_vec(); 3]);
        }

        #[test]
        fn viewport_pacer_is_event_driven_and_keeps_one_deadline() {
            let start = Instant::now();
            let mut pacer = ViewportPresentationPacer::default();
            assert_eq!(pacer.deadline(), None);

            pacer.mark_dirty(start);
            assert_eq!(pacer.deadline(), Some(start));
            pacer.mark_dirty(start + Duration::from_millis(8));
            assert_eq!(
                pacer.deadline(),
                Some(start),
                "a cold dirty deadline must not slide before its first presentation"
            );
            assert!(pacer.due(start));
            pacer.mark_presented(start);
            assert_eq!(pacer.deadline(), None);

            pacer.mark_dirty(start + Duration::from_millis(1));
            assert_eq!(
                pacer.deadline(),
                Some(start + MIN_VIEWPORT_PRESENT_INTERVAL)
            );
            pacer.mark_dirty(start + Duration::from_millis(8));
            assert_eq!(
                pacer.deadline(),
                Some(start + MIN_VIEWPORT_PRESENT_INTERVAL),
                "additional dirtiness must not create or slide a second deadline"
            );
            assert!(!pacer.due(start + Duration::from_millis(15)));
            assert!(pacer.due(start + MIN_VIEWPORT_PRESENT_INTERVAL));

            pacer.cancel();
            assert_eq!(pacer.deadline(), None);
            assert!(!pacer.due(start + Duration::from_secs(1)));
        }

        #[test]
        fn one_stdin_wheel_burst_presents_only_the_final_cached_offset() {
            let (mut viewport, physical) = cached_window_viewport();
            let mut codec = HostInputCodec::new();
            let events = codec
                .feed(b"\x1b[<64;5;3M\x1b[<64;5;3M\x1b[<64;5;3M")
                .expect("one stdin delivery decodes all wheel reports");
            assert_eq!(events.len(), 3);

            let now = Instant::now();
            let mut pacer = ViewportPresentationPacer::default();
            for event in events {
                let HostInputEvent::Mouse(mouse) = event else {
                    panic!("wheel burst must contain only mouse reports");
                };
                assert_eq!(
                    route_mouse_to_child(&mouse, ActiveScreen::Main, TerminalModes::default(),),
                    None
                );
                assert!(matches!(
                    viewport.navigate(mouse.wheel_is_up(), 1),
                    ViewportEffect::Render | ViewportEffect::RenderAndRequestHistoryWindow(_)
                ));
                assert!(mark_cached_viewport_dirty(&viewport, &mut pacer, now));
            }
            assert_eq!(viewport.window_cache.desired_offset_from_bottom(), 4);

            let mut output = ViewportFrameWriter::default();
            let mut status = StatusRenderer::new(None, physical);
            assert!(
                present_cached_viewport_with_writer(
                    &mut output,
                    &mut viewport,
                    &mut status,
                    TerminalViewTransportState::Active,
                    &mut pacer,
                    now,
                    false,
                )
                .expect("present one coalesced wheel frame")
            );
            assert_eq!(output.writes, 1);
            assert_eq!(output.flushes, 1);
            assert_eq!(
                output
                    .bytes
                    .windows(HOST_SYNC_BEGIN.len())
                    .filter(|window| *window == HOST_SYNC_BEGIN)
                    .count(),
                1
            );
            assert_eq!(
                viewport
                    .scroll_metrics()
                    .map(|metrics| metrics.offset_from_bottom),
                Some(4)
            );
            assert!(matches!(
                viewport.start_resume(Vec::new()),
                ViewportEffect::Resume
            ));
            assert_eq!(
                viewport
                    .scroll_metrics()
                    .map(|metrics| metrics.offset_from_bottom),
                Some(4),
                "a successfully emitted paced frame becomes the resume presentation baseline"
            );
        }

        #[test]
        fn cross_delivery_pacing_keeps_latest_reverse_offset_and_cancels_a_miss() {
            let (mut viewport, physical) = cached_window_viewport();
            let start = Instant::now();
            let mut pacer = ViewportPresentationPacer::default();
            pacer.mark_presented(start);
            let mut output = ViewportFrameWriter::default();
            let mut status = StatusRenderer::new(None, physical);

            for (older, elapsed) in [(true, 1), (true, 4), (false, 8)] {
                assert!(matches!(
                    viewport.navigate(older, 1),
                    ViewportEffect::Render | ViewportEffect::RenderAndRequestHistoryWindow(_)
                ));
                let now = start + Duration::from_millis(elapsed);
                assert!(mark_cached_viewport_dirty(&viewport, &mut pacer, now));
                assert!(
                    !present_cached_viewport_with_writer(
                        &mut output,
                        &mut viewport,
                        &mut status,
                        TerminalViewTransportState::Active,
                        &mut pacer,
                        now,
                        false,
                    )
                    .expect("pre-deadline deliveries remain queued")
                );
            }
            assert_eq!(viewport.window_cache.desired_offset_from_bottom(), 2);
            assert!(
                present_cached_viewport_with_writer(
                    &mut output,
                    &mut viewport,
                    &mut status,
                    TerminalViewTransportState::Active,
                    &mut pacer,
                    start + MIN_VIEWPORT_PRESENT_INTERVAL,
                    false,
                )
                .expect("deadline presents the latest reverse-direction target")
            );
            assert_eq!(output.writes, 1);
            assert_eq!(output.flushes, 1);

            assert!(matches!(
                viewport.navigate(true, 1),
                ViewportEffect::Render | ViewportEffect::RenderAndRequestHistoryWindow(_)
            ));
            assert!(mark_cached_viewport_dirty(
                &viewport,
                &mut pacer,
                start + Duration::from_millis(17),
            ));
            assert!(
                !present_cached_viewport_with_writer(
                    &mut output,
                    &mut viewport,
                    &mut status,
                    TerminalViewTransportState::Active,
                    &mut pacer,
                    start + Duration::from_millis(31),
                    false,
                )
                .expect("a new delivery after one frame waits for the next cadence boundary")
            );
            assert!(
                present_cached_viewport_with_writer(
                    &mut output,
                    &mut viewport,
                    &mut status,
                    TerminalViewTransportState::Active,
                    &mut pacer,
                    start + Duration::from_millis(32),
                    false,
                )
                .expect("a burst crossing the boundary produces one later paced frame")
            );
            assert_eq!((output.writes, output.flushes), (2, 2));
            assert_eq!(viewport.window_cache.desired_offset_from_bottom(), 3);

            assert!(matches!(
                viewport.navigate(true, 1),
                ViewportEffect::Render | ViewportEffect::RenderAndRequestHistoryWindow(_)
            ));
            assert!(mark_cached_viewport_dirty(
                &viewport,
                &mut pacer,
                start + Duration::from_millis(33),
            ));
            viewport.window_cache.defer_pending_request();
            assert!(matches!(
                viewport.scroll_to_offset(10),
                ViewportEffect::RequestHistoryWindow(query)
                    if query.target_offset_from_bottom == 10
            ));
            cancel_unpresentable_cached_viewport(&viewport, &mut pacer);
            assert_eq!(pacer.deadline(), None);
            assert_eq!(
                viewport
                    .scroll_metrics()
                    .map(|metrics| metrics.offset_from_bottom),
                Some(3),
                "a miss must retain the last emitted offset, not an unpainted cache hit"
            );
            let mut miss_repaint = Vec::new();
            write_history(&mut miss_repaint, &viewport)
                .expect("a miss leaves the existing physical content untouched");
            assert!(miss_repaint.is_empty());
            assert!(
                !present_cached_viewport_with_writer(
                    &mut output,
                    &mut viewport,
                    &mut status,
                    TerminalViewTransportState::Active,
                    &mut pacer,
                    start + Duration::from_secs(1),
                    false,
                )
                .expect("a cache miss cannot repaint the previous target")
            );
            assert_eq!(output.writes, 2);
        }

        #[test]
        fn drag_release_forces_the_latest_complete_cached_frame() {
            let (mut viewport, physical) = cached_window_viewport();
            let start = Instant::now();
            let mut pacer = ViewportPresentationPacer::default();
            pacer.mark_presented(start);
            assert!(matches!(
                viewport.navigate(true, 1),
                ViewportEffect::Render | ViewportEffect::RenderAndRequestHistoryWindow(_)
            ));
            assert!(mark_cached_viewport_dirty(
                &viewport,
                &mut pacer,
                start + Duration::from_millis(1),
            ));

            viewport.drag_grab_row = Some(0);
            let release = SgrMouse {
                code: 0,
                column: 1,
                row: 1,
                release: true,
                raw: b"\x1b[<0;1;1m".to_vec(),
            };
            assert!(matches!(
                viewport.handle_gutter_mouse_at(&release, true, start + Duration::from_millis(2),),
                Some(ViewportEffect::None)
            ));

            let mut output = ViewportFrameWriter::default();
            let mut status = StatusRenderer::new(None, physical);
            assert!(
                present_cached_viewport_with_writer(
                    &mut output,
                    &mut viewport,
                    &mut status,
                    TerminalViewTransportState::Active,
                    &mut pacer,
                    start + Duration::from_millis(2),
                    true,
                )
                .expect("release bypasses the remaining presentation interval")
            );
            assert_eq!((output.writes, output.flushes), (1, 1));
        }

        #[test]
        fn compatible_background_delta_keeps_a_pending_cached_presentation() {
            let (mut viewport, physical) = cached_window_viewport();
            let start = Instant::now();
            let mut pacer = ViewportPresentationPacer::default();
            pacer.mark_presented(start);
            assert!(matches!(
                viewport.navigate(true, 1),
                ViewportEffect::Render | ViewportEffect::RenderAndRequestHistoryWindow(_)
            ));
            assert!(mark_cached_viewport_dirty(
                &viewport,
                &mut pacer,
                start + Duration::from_millis(1),
            ));
            let deadline = pacer.deadline();

            let previous = viewport.live_metrics.expect("fixture live metrics");
            viewport.observe_live_metrics(Some(TerminalScrollMetrics {
                revision: Revision::new(previous.revision.get() + 1),
                max_offset_from_bottom: previous.max_offset_from_bottom + 1,
                ..previous
            }));
            cancel_unpresentable_cached_viewport(&viewport, &mut pacer);
            assert_eq!(pacer.deadline(), deadline);
            assert_eq!(
                viewport.window_cache.desired_offset_from_bottom(),
                3,
                "same-epoch growth translates the pending target to keep its content pinned"
            );

            let mut output = ViewportFrameWriter::default();
            let mut status = StatusRenderer::new(None, physical);
            assert!(
                present_cached_viewport_with_writer(
                    &mut output,
                    &mut viewport,
                    &mut status,
                    TerminalViewTransportState::Active,
                    &mut pacer,
                    start + MIN_VIEWPORT_PRESENT_INTERVAL,
                    false,
                )
                .expect("compatible background output keeps the latest complete frame due")
            );
            assert_eq!((output.writes, output.flushes), (1, 1));
        }

        #[test]
        fn cancelled_viewport_deadline_cannot_repaint_after_resume_or_resize() {
            let (mut viewport, physical) = cached_window_viewport();
            let start = Instant::now();
            let mut pacer = ViewportPresentationPacer::default();
            pacer.mark_presented(start);
            assert!(matches!(
                viewport.navigate(true, 1),
                ViewportEffect::Render | ViewportEffect::RenderAndRequestHistoryWindow(_)
            ));
            assert!(mark_cached_viewport_dirty(
                &viewport,
                &mut pacer,
                start + Duration::from_millis(1),
            ));
            assert_eq!(
                viewport
                    .scroll_metrics()
                    .map(|metrics| metrics.offset_from_bottom),
                Some(2),
                "the desired cached target advances before presentation"
            );

            assert!(matches!(
                viewport.start_resume(Vec::new()),
                ViewportEffect::Resume
            ));
            assert_eq!(
                viewport
                    .scroll_metrics()
                    .map(|metrics| metrics.offset_from_bottom),
                Some(1),
                "resume must preserve the last actually presented scrollbar, not the pending target"
            );
            pacer.cancel();
            let mut output = ViewportFrameWriter::default();
            let mut status = StatusRenderer::new(None, physical);
            assert!(
                !present_cached_viewport_with_writer(
                    &mut output,
                    &mut viewport,
                    &mut status,
                    TerminalViewTransportState::Synchronizing,
                    &mut pacer,
                    start + MIN_VIEWPORT_PRESENT_INTERVAL,
                    false,
                )
                .expect("resume invalidates the pending history frame")
            );
            assert_eq!((output.writes, output.flushes), (0, 0));

            let (mut resized, _) = cached_window_viewport();
            let mut resize_pacer = ViewportPresentationPacer::default();
            resize_pacer.mark_dirty(start);
            resized.resize(TerminalSize::new(5, 9));
            cancel_unpresentable_cached_viewport(&resized, &mut resize_pacer);
            assert_eq!(resize_pacer.deadline(), None);
        }

        #[test]
        fn bracketed_paste_overflow_fails_without_emitting_a_partial_input_event() {
            let mut codec = HostInputCodec::new();
            assert!(
                codec
                    .feed(PASTE_START)
                    .expect("paste prefix is within the bound")
                    .is_empty()
            );
            let Err(CliError::Daemon(error)) = codec.feed(&vec![b'x'; RESUME_INPUT_BOUND]) else {
                panic!("unterminated paste beyond the fixed bound must fail with a daemon error");
            };
            assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);
        }

        #[test]
        fn viewport_resumes_authoritative_live_state_and_forwards_retained_input_once() {
            let mut viewport = ViewportController::new(TerminalSize::new(2, 20));
            let ViewportEffect::Request(request) = viewport.navigate(true, 1) else {
                panic!("the first upward gesture must request newest retained history");
            };
            assert_eq!(request.direction, TerminalHistoryDirection::Newest);
            assert_eq!(request.cursor, None);
            assert!(viewport.is_history());

            let cursor = TerminalHistoryCursor {
                epoch: Revision::new(3),
                revision: Revision::new(7),
                start_row: 0,
                row_count: 3,
                oldest_row: 0,
                newest_row: 3,
            };
            viewport
                .apply_history(TerminalHistoryResult::Page(TerminalHistoryPage {
                    cursor,
                    rows: vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()],
                }))
                .expect("apply one authoritative history page");
            let (visible, height, notice) = viewport
                .visible_history_rows()
                .expect("history remains pinned");
            assert_eq!(visible, &[b"two".to_vec(), b"three".to_vec()]);
            assert_eq!(height, 2);
            assert_eq!(notice, None);

            assert!(matches!(
                viewport
                    .retain_or_resume(b"key".to_vec())
                    .expect("normal input begins bounded resume"),
                ViewportEffect::Resume
            ));
            assert_eq!(
                viewport.scroll_metrics(),
                None,
                "the pager protocol must not invent scrollbar metrics while resuming"
            );
            viewport
                .retain_resume_input(b"-paste")
                .expect("paste bytes stay bounded while the snapshot is pending");
            assert_eq!(viewport.finish_resume(), None);
            viewport.observe_snapshot(None);
            assert_eq!(viewport.finish_resume(), Some(b"key-paste".to_vec()));
            assert_eq!(viewport.finish_resume(), None);
            assert!(viewport.is_live());
        }

        #[test]
        fn ordinary_input_resume_preserves_legacy_metrics_until_snapshot_or_resize() {
            let layout = ChromeLayout::new(TerminalSize::new(3, 10), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(2),
                revision: Revision::new(7),
                offset_from_bottom: 0,
                max_offset_from_bottom: 10,
                viewport_rows: 3,
            };
            let history = TerminalScrollMetrics {
                offset_from_bottom: 4,
                ..live
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            viewport.window_supported = false;
            assert!(matches!(
                viewport.navigate(true, 4),
                ViewportEffect::RequestViewport(_)
            ));
            assert!(matches!(
                viewport
                    .apply_viewport(TerminalViewportResult::Frame(TerminalViewportFrame {
                        disposition: TerminalViewportDisposition::Exact,
                        metrics: history,
                        rows: vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()],
                    }))
                    .expect("complete legacy viewport frame"),
                ViewportEffect::Render
            ));
            let mut status = StatusRenderer::new(None, TerminalSize::new(3, 10));
            render_history_with_chrome(
                &mut Vec::new(),
                &viewport,
                &mut status,
                TerminalViewTransportState::Active,
            )
            .expect("present the initial legacy viewport frame");
            viewport.observe_presentation();
            assert!(matches!(
                viewport.navigate(true, 1),
                ViewportEffect::RequestViewport(TerminalScrollAction::ScrollByLines(1))
            ));
            assert!(matches!(viewport.navigate(true, 1), ViewportEffect::None));
            assert!(matches!(
                viewport
                    .apply_viewport(TerminalViewportResult::Frame(TerminalViewportFrame {
                        disposition: TerminalViewportDisposition::Exact,
                        metrics: TerminalScrollMetrics {
                            offset_from_bottom: 5,
                            ..history
                        },
                        rows: vec![b"older".to_vec(), b"middle".to_vec(), b"newer".to_vec()],
                    }))
                    .expect("coalesced intermediate semantic frame"),
                ViewportEffect::RequestViewport(TerminalScrollAction::ScrollByLines(1))
            ));
            assert_eq!(
                viewport.scroll_metrics(),
                Some(history),
                "an unpainted coalesced frame must not replace presented geometry"
            );

            assert!(matches!(
                viewport
                    .retain_or_resume(b"input".to_vec())
                    .expect("ordinary input starts one authoritative resume"),
                ViewportEffect::Resume
            ));
            assert_eq!(viewport.scroll_metrics(), Some(history));

            viewport.resize(TerminalSize::new(4, 9));
            assert_eq!(
                viewport.scroll_metrics(),
                None,
                "old metrics cannot be projected onto resized chrome"
            );
            viewport.observe_snapshot(Some(TerminalScrollMetrics {
                epoch: Revision::new(9),
                revision: Revision::new(8),
                viewport_rows: 4,
                ..live
            }));
            assert_eq!(
                viewport.scroll_metrics(),
                None,
                "structurally invalid replacement metrics never reach chrome"
            );
            let resized_live = TerminalScrollMetrics {
                revision: Revision::new(8),
                viewport_rows: 4,
                ..live
            };
            viewport.observe_snapshot(Some(resized_live));
            assert_eq!(viewport.scroll_metrics(), Some(resized_live));
            assert_eq!(viewport.finish_resume(), Some(b"input".to_vec()));
        }

        #[test]
        fn loading_and_resume_pending_never_emit_an_intermediate_content_frame() {
            let mut viewport = ViewportController::new(TerminalSize::new(2, 20));
            assert!(matches!(
                viewport.navigate(true, 1),
                ViewportEffect::Request(_)
            ));
            let mut loading = Vec::new();
            write_history(&mut loading, &viewport).expect("loading render is a no-op");
            assert!(loading.is_empty());

            viewport
                .apply_history(TerminalHistoryResult::Page(TerminalHistoryPage {
                    cursor: TerminalHistoryCursor {
                        epoch: Revision::new(1),
                        revision: Revision::new(2),
                        start_row: 0,
                        row_count: 2,
                        oldest_row: 0,
                        newest_row: 2,
                    },
                    rows: vec![b"old-a".to_vec(), b"old-b".to_vec()],
                }))
                .expect("complete legacy page");
            let mut complete = Vec::new();
            write_history(&mut complete, &viewport).expect("complete frame renders");
            assert!(contains_bytes(&complete, b"old-a"));

            assert!(matches!(
                viewport.start_resume(Vec::new()),
                ViewportEffect::Resume
            ));
            let mut returning = Vec::new();
            write_history(&mut returning, &viewport).expect("resume-pending render is a no-op");
            assert!(returning.is_empty());
        }

        #[test]
        fn semantic_viewport_coalesces_one_pending_request_and_resumes_live_once() {
            let layout = ChromeLayout::new(TerminalSize::new(3, 10), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(2),
                revision: Revision::new(7),
                offset_from_bottom: 0,
                max_offset_from_bottom: 10,
                viewport_rows: 3,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            viewport.window_supported = false;
            assert!(matches!(
                viewport.navigate(true, 3),
                ViewportEffect::RequestViewport(TerminalScrollAction::ScrollByLines(3))
            ));
            assert!(matches!(viewport.navigate(true, 4), ViewportEffect::None));

            let first = TerminalViewportFrame {
                disposition: zterm_core::terminal::TerminalViewportDisposition::Exact,
                metrics: TerminalScrollMetrics {
                    offset_from_bottom: 3,
                    ..live
                },
                rows: vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()],
            };
            assert!(matches!(
                viewport
                    .apply_viewport(TerminalViewportResult::Frame(first))
                    .expect("correlated semantic frame"),
                ViewportEffect::RequestViewport(TerminalScrollAction::ScrollByLines(4))
            ));

            let second = TerminalViewportFrame {
                disposition: zterm_core::terminal::TerminalViewportDisposition::Exact,
                metrics: TerminalScrollMetrics {
                    offset_from_bottom: 7,
                    ..live
                },
                rows: vec![b"older".to_vec(), b"middle".to_vec(), b"newer".to_vec()],
            };
            assert!(matches!(
                viewport
                    .apply_viewport(TerminalViewportResult::Frame(second))
                    .expect("second correlated semantic frame"),
                ViewportEffect::Render
            ));
            assert_eq!(
                viewport.visible_history_rows().map(|visible| visible.0),
                Some(&[b"older".to_vec(), b"middle".to_vec(), b"newer".to_vec()][..])
            );
            assert!(matches!(
                viewport.navigate(false, 7),
                ViewportEffect::RequestViewport(TerminalScrollAction::ScrollByLines(-7))
            ));
            assert!(matches!(
                viewport
                    .apply_viewport(TerminalViewportResult::Live(live))
                    .expect("live outcome begins authoritative resume"),
                ViewportEffect::Resume
            ));
            viewport.observe_snapshot(Some(live));
            assert_eq!(viewport.finish_resume(), Some(Vec::new()));
            assert_eq!(viewport.finish_resume(), None);
        }

        #[test]
        fn history_window_coalesces_latest_target_and_moves_locally_by_one_line() {
            let layout = ChromeLayout::new(TerminalSize::new(3, 10), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(2),
                revision: Revision::new(7),
                offset_from_bottom: 0,
                max_offset_from_bottom: 10,
                viewport_rows: 3,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            let ViewportEffect::RequestHistoryWindow(first) = viewport.navigate(true, 1) else {
                panic!("one host wheel report should request one absolute history line");
            };
            assert_eq!(first.target_offset_from_bottom, 1);
            assert!(matches!(viewport.navigate(true, 2), ViewportEffect::None));

            let rows = (-7_i64..2)
                .map(|row| row.to_string().into_bytes())
                .collect::<Vec<_>>();
            assert!(matches!(
                viewport
                    .apply_history_window(TerminalHistoryWindowResult::Frame(
                        zterm_core::terminal::TerminalHistoryWindowFrame {
                            disposition: TerminalViewportDisposition::Exact,
                            anchor: TerminalHistoryWindowAnchor {
                                epoch: live.epoch,
                                revision: live.revision,
                                max_offset_from_bottom: live.max_offset_from_bottom,
                                viewport: layout.child,
                            },
                            target_offset_from_bottom: 1,
                            first_row_from_live_top: -7,
                            ansi_rows: rows,
                        },
                    ))
                    .expect("valid coalesced history window"),
                ViewportEffect::Render
            ));
            assert_eq!(
                viewport.visible_history_rows().map(|visible| visible.0),
                Some(&[b"-3".to_vec(), b"-2".to_vec(), b"-1".to_vec()][..])
            );
            assert!(matches!(viewport.navigate(true, 1), ViewportEffect::Render));
            assert_eq!(viewport.window_cache.desired_offset_from_bottom(), 4);
            assert_eq!(
                viewport.visible_history_rows().map(|visible| visible.0),
                Some(&[b"-4".to_vec(), b"-3".to_vec(), b"-2".to_vec()][..])
            );
        }

        #[test]
        fn history_window_resume_never_presents_a_blank_gutter_before_live_snapshot() {
            let physical = TerminalSize::new(4, 10);
            let layout = ChromeLayout::new(physical, false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(2),
                revision: Revision::new(7),
                offset_from_bottom: 0,
                max_offset_from_bottom: 12,
                viewport_rows: layout.child.rows,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            let ViewportEffect::RequestHistoryWindow(query) = viewport.navigate(true, 4) else {
                panic!("the uncached history target must request one window");
            };
            let shape = query
                .response_shape(query.anchor)
                .expect("controller query has one valid response shape");
            assert!(matches!(
                viewport
                    .apply_history_window(TerminalHistoryWindowResult::Frame(
                        zterm_core::terminal::TerminalHistoryWindowFrame {
                            disposition: shape.disposition,
                            anchor: query.anchor,
                            target_offset_from_bottom: shape.target_offset_from_bottom,
                            first_row_from_live_top: shape.first_row_from_live_top,
                            ansi_rows: vec![b"row".to_vec(); shape.row_count],
                        },
                    ))
                    .expect("complete cached history window"),
                ViewportEffect::Render
            ));
            let history_metrics = TerminalScrollMetrics {
                offset_from_bottom: 4,
                ..live
            };
            assert_eq!(viewport.scroll_metrics(), Some(history_metrics));

            let mut status = StatusRenderer::new(None, physical);
            let mut history_output = Vec::new();
            render_history_with_chrome(
                &mut history_output,
                &viewport,
                &mut status,
                TerminalViewTransportState::Active,
            )
            .expect("present the complete cached history frame");
            viewport.observe_presentation();
            assert!(contains_bytes(&history_output, b"row"));

            assert!(matches!(
                viewport.navigate(false, 4),
                ViewportEffect::Resume
            ));
            assert_eq!(
                viewport.scroll_metrics(),
                Some(history_metrics),
                "ResumePending retains the scrollbar which is still on screen"
            );
            viewport.observe_sync_required();
            let mut sync_frame = Vec::new();
            render_history_with_chrome(
                &mut sync_frame,
                &viewport,
                &mut status,
                TerminalViewTransportState::Synchronizing,
            )
            .expect("repair chrome while the replacement snapshot is in flight");
            viewport.observe_presentation();
            assert!(contains_bytes(&sync_frame, "▐".as_bytes()));
            assert!(contains_bytes(&sync_frame, "▕".as_bytes()));

            let replacement_live = TerminalScrollMetrics {
                revision: Revision::new(8),
                ..live
            };
            viewport.observe_snapshot(Some(replacement_live));
            assert_eq!(viewport.scroll_metrics(), Some(replacement_live));
            let mut snapshot_output = Vec::new();
            let mut renderer = TerminalRenderer::new();
            renderer
                .apply_snapshot_with_chrome(
                    &mut snapshot_output,
                    RenderSnapshot {
                        revision: replacement_live.revision,
                        active_screen: ActiveScreen::Main,
                        modes: TerminalModes::default(),
                        recent_history_ansi: b"",
                        screen_ansi: b"\x1b[?1049llive",
                    },
                    |writer| {
                        write_chrome(
                            writer,
                            &viewport,
                            &mut status,
                            TerminalViewTransportState::Synchronizing,
                        )
                    },
                )
                .expect("atomically present the replacement snapshot and live chrome");
            viewport.observe_presentation();

            let mut expected = HOST_SYNC_BEGIN.to_vec();
            expected.extend_from_slice(b"live");
            let expected_viewport = ViewportController::with_layout(layout, Some(replacement_live));
            let mut expected_status = StatusRenderer::new(None, physical);
            write_chrome(
                &mut expected,
                &expected_viewport,
                &mut expected_status,
                TerminalViewTransportState::Synchronizing,
            )
            .expect("build exact live-bottom chrome");
            expected.extend_from_slice(HOST_INPUT_CAPTURE);
            expected.extend_from_slice(HOST_SYNC_END);
            assert_eq!(snapshot_output, expected);

            assert_eq!(viewport.finish_resume(), Some(Vec::new()));
            assert!(viewport.is_live());
            assert_eq!(viewport.scroll_metrics(), Some(replacement_live));
            let mut active_frame = Vec::new();
            render_scrollbar(&mut active_frame, &mut viewport)
                .expect("the Active transition retains live-bottom chrome");
            assert!(contains_bytes(&active_frame, "▐".as_bytes()));
            assert!(contains_bytes(&active_frame, "▕".as_bytes()));
        }

        #[test]
        fn remote_history_resume_keeps_connection_status_stable_across_every_frame() {
            let physical = TerminalSize::new(10, 30);
            let layout = ChromeLayout::new(physical, true, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(2),
                revision: Revision::new(7),
                offset_from_bottom: 0,
                max_offset_from_bottom: 18,
                viewport_rows: layout.child.rows,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            let ViewportEffect::RequestHistoryWindow(query) = viewport.navigate(true, 6) else {
                panic!("the uncached history target must request one window");
            };
            let shape = query
                .response_shape(query.anchor)
                .expect("controller query has one valid response shape");
            assert!(matches!(
                viewport
                    .apply_history_window(TerminalHistoryWindowResult::Frame(
                        zterm_core::terminal::TerminalHistoryWindowFrame {
                            disposition: shape.disposition,
                            anchor: query.anchor,
                            target_offset_from_bottom: shape.target_offset_from_bottom,
                            first_row_from_live_top: shape.first_row_from_live_top,
                            ansi_rows: vec![b"history-row".to_vec(); shape.row_count],
                        },
                    ))
                    .expect("complete cached history window"),
                ViewportEffect::Render
            ));

            let mut status = StatusRenderer::new(Some("remote".to_owned()), physical);
            status.path = TerminalViewConnectionPath::Direct;
            status.rtt_ms = Some(11);
            let active_status = b"remote | direct | 11 ms";
            let inactive_status = b"remote | -- | --";
            let mut output = ViewportFrameWriter::default();

            render_history_with_chrome(
                &mut output,
                &viewport,
                &mut status,
                TerminalViewTransportState::Active,
            )
            .expect("present history with the known connection status");
            viewport.observe_presentation();
            assert_eq!(output.frames.len(), 1);
            assert!(contains_bytes(&output.frames[0], "\x1b[5;30H▐".as_bytes()));

            assert!(matches!(
                viewport.navigate(false, 6),
                ViewportEffect::Resume
            ));
            assert!(
                !render_transport_transition_view_with_writer(
                    &mut output,
                    &viewport,
                    &mut status,
                    TerminalViewTransportState::Synchronizing,
                    false,
                )
                .expect("the in-epoch synchronizing transition is visually silent")
            );
            viewport.observe_sync_required();
            assert_eq!(
                output.frames.len(),
                1,
                "Synchronizing and SyncRequired retain the last complete frame"
            );

            let replacement_live = TerminalScrollMetrics {
                revision: Revision::new(8),
                ..live
            };
            viewport.observe_snapshot(Some(replacement_live));
            let mut renderer = TerminalRenderer::new();
            renderer
                .apply_snapshot_with_chrome(
                    &mut output,
                    RenderSnapshot {
                        revision: replacement_live.revision,
                        active_screen: ActiveScreen::Main,
                        modes: TerminalModes::default(),
                        recent_history_ansi: b"",
                        screen_ansi: b"\x1b[?1049llive-row",
                    },
                    |writer| {
                        write_chrome(
                            writer,
                            &viewport,
                            &mut status,
                            TerminalViewTransportState::Synchronizing,
                        )
                    },
                )
                .expect("present one authoritative live replacement");
            viewport.observe_presentation();

            assert_eq!(viewport.finish_resume(), Some(Vec::new()));
            assert!(
                !render_transport_transition_view_with_writer(
                    &mut output,
                    &viewport,
                    &mut status,
                    TerminalViewTransportState::Active,
                    true,
                )
                .expect("the post-snapshot Active transition is visually silent")
            );

            assert_eq!(output.frames.len(), 2);
            for frame in &output.frames {
                assert!(
                    contains_bytes(frame, active_status),
                    "every in-epoch frame must retain the known connection status"
                );
                assert!(
                    !contains_bytes(frame, inactive_status),
                    "an in-epoch visual sync must not project a false disconnect"
                );
            }
            assert!(contains_bytes(&output.frames[0], b"history-row"));
            assert!(!contains_bytes(&output.frames[0], b"live-row"));
            assert!(contains_bytes(&output.frames[1], b"live-row"));
            assert!(
                contains_bytes(&output.frames[1], "\x1b[7;30H▐".as_bytes()),
                "the replacement snapshot paints the offset-zero thumb in its own transaction"
            );
            for frame in &output.frames {
                assert!(frame.starts_with(HOST_SYNC_BEGIN));
                assert!(contains_bytes(frame, HOST_INPUT_CAPTURE));
                assert!(frame.ends_with(HOST_SYNC_END));
            }
            assert_eq!(output.writes, 2);
            assert_eq!(output.flushes, 2);
        }

        #[test]
        fn unsupported_history_window_falls_back_to_existing_semantic_viewport() {
            let layout = ChromeLayout::new(TerminalSize::new(3, 10), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                offset_from_bottom: 0,
                max_offset_from_bottom: 6,
                viewport_rows: 3,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            assert!(matches!(
                viewport.navigate(true, 1),
                ViewportEffect::RequestHistoryWindow(_)
            ));
            assert!(matches!(
                viewport
                    .apply_history_window(TerminalHistoryWindowResult::HistoryGap {
                        epoch: Revision::new(0),
                        revision: Revision::new(0),
                    })
                    .expect("capability gap is a negotiated fallback"),
                ViewportEffect::RequestViewport(TerminalScrollAction::ScrollToOffset(1))
            ));
            assert!(!viewport.window_supported);
            assert!(matches!(
                viewport
                    .apply_viewport(TerminalViewportResult::HistoryGap {
                        epoch: Revision::ZERO,
                        revision: Revision::ZERO,
                    })
                    .expect("missing viewport capability falls back once more"),
                ViewportEffect::Request(HistoryRequest {
                    direction: TerminalHistoryDirection::Newest,
                    cursor: None,
                })
            ));
            assert!(matches!(
                viewport.state,
                ViewportState::History(HistoryViewport {
                    protocol: HistoryProtocol::Pager,
                    ..
                })
            ));
        }

        #[test]
        fn transport_loss_gap_keeps_window_capability_and_does_not_repaint_old_content() {
            let layout = ChromeLayout::new(TerminalSize::new(3, 10), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(2),
                revision: Revision::new(7),
                offset_from_bottom: 0,
                max_offset_from_bottom: 10,
                viewport_rows: 3,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            assert!(matches!(
                viewport.navigate(true, 1),
                ViewportEffect::RequestHistoryWindow(_)
            ));
            viewport
                .apply_history_window(TerminalHistoryWindowResult::Frame(
                    zterm_core::terminal::TerminalHistoryWindowFrame {
                        disposition: TerminalViewportDisposition::Exact,
                        anchor: TerminalHistoryWindowAnchor {
                            epoch: live.epoch,
                            revision: live.revision,
                            max_offset_from_bottom: live.max_offset_from_bottom,
                            viewport: layout.child,
                        },
                        target_offset_from_bottom: 1,
                        first_row_from_live_top: -7,
                        ansi_rows: (-7_i64..2)
                            .map(|row| row.to_string().into_bytes())
                            .collect(),
                    },
                ))
                .expect("complete initial window");
            assert!(matches!(
                viewport.scroll_to_offset(9),
                ViewportEffect::RequestHistoryWindow(_)
            ));

            assert!(matches!(
                viewport
                    .apply_history_window(TerminalHistoryWindowResult::HistoryGap {
                        epoch: Revision::new(2),
                        revision: Revision::new(7),
                    })
                    .expect("correlated stream-loss gap"),
                ViewportEffect::None
            ));
            assert!(viewport.window_supported);
            assert!(matches!(
                viewport.state,
                ViewportState::History(HistoryViewport {
                    protocol: HistoryProtocol::Window,
                    ..
                })
            ));
            let mut repaint = Vec::new();
            write_history(&mut repaint, &viewport).expect("content-free gap render is a no-op");
            assert!(repaint.is_empty());
        }

        #[test]
        fn resized_pinned_window_refetches_only_after_authoritative_anchor_update() {
            let layout = ChromeLayout::new(TerminalSize::new(3, 10), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(2),
                revision: Revision::new(7),
                offset_from_bottom: 0,
                max_offset_from_bottom: 10,
                viewport_rows: 3,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            assert!(matches!(
                viewport.navigate(true, 1),
                ViewportEffect::RequestHistoryWindow(_)
            ));
            viewport
                .apply_history_window(TerminalHistoryWindowResult::Frame(
                    zterm_core::terminal::TerminalHistoryWindowFrame {
                        disposition: TerminalViewportDisposition::Exact,
                        anchor: TerminalHistoryWindowAnchor {
                            epoch: live.epoch,
                            revision: live.revision,
                            max_offset_from_bottom: live.max_offset_from_bottom,
                            viewport: layout.child,
                        },
                        target_offset_from_bottom: 1,
                        first_row_from_live_top: -7,
                        ansi_rows: vec![b"row".to_vec(); 9],
                    },
                ))
                .expect("complete initial window");

            viewport.resize(TerminalSize::new(4, 10));
            assert!(viewport.window_cache.visible_rows().is_none());
            assert!(!viewport.window_cache.request_pending());
            let resized_live = TerminalScrollMetrics {
                revision: Revision::new(8),
                viewport_rows: 4,
                ..live
            };
            viewport.observe_snapshot(Some(resized_live));
            let refill = viewport
                .refetch_history_window()
                .expect("authoritative resized anchor triggers one replacement query");
            assert_eq!(refill.anchor.viewport, TerminalSize::new(4, 10));
            assert_eq!(refill.anchor.revision, Revision::new(8));
            assert_eq!(refill.target_offset_from_bottom, 1);
        }

        #[test]
        fn drag_motion_is_paced_but_release_submits_the_final_absolute_target() {
            let layout = ChromeLayout::new(TerminalSize::new(4, 10), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                offset_from_bottom: 0,
                max_offset_from_bottom: 12,
                viewport_rows: 4,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            viewport.state = ViewportState::History(HistoryViewport {
                protocol: HistoryProtocol::Window,
                page: None,
                frame: None,
                offset: 0,
                pending: None,
                viewport_pending: false,
                queued_scroll: None,
                notice: None,
            });
            let now = Instant::now();
            viewport.drag_grab_row = Some(0);
            assert!(viewport.window_cache.set_target(4).request.is_some());
            viewport.window_cache.defer_pending_request();
            viewport.drag_deferred_target = Some(4);
            let unchanged_motion = SgrMouse {
                code: 32,
                column: 10,
                row: 3,
                release: false,
                raw: b"\x1b[<32;10;3M".to_vec(),
            };
            assert!(matches!(
                viewport.handle_gutter_mouse_at(&unchanged_motion, true, now),
                Some(ViewportEffect::None)
            ));
            assert_eq!(viewport.drag_deferred_target, Some(4));

            viewport.drag_last_request = Some(now);
            let update = viewport.window_cache.set_target(5);
            let paced = viewport.pace_drag_effect(
                history_window_effect(update),
                5,
                now + Duration::from_millis(1),
            );
            assert!(matches!(paced, ViewportEffect::None));
            assert!(!viewport.window_cache.request_pending());

            let release = SgrMouse {
                code: 0,
                column: 1,
                row: 1,
                release: true,
                raw: b"\x1b[<0;1;1m".to_vec(),
            };
            assert!(matches!(
                viewport.handle_gutter_mouse_at(
                    &release,
                    true,
                    now + Duration::from_millis(2)
                ),
                Some(ViewportEffect::RequestHistoryWindow(query))
                    if query.target_offset_from_bottom == 5
            ));

            viewport.window_cache.defer_pending_request();
            viewport.drag_last_request = Some(now);
            let update = viewport.window_cache.set_target(6);
            assert!(matches!(
                viewport.pace_drag_effect(
                    history_window_effect(update),
                    6,
                    now + DRAG_REQUEST_INTERVAL,
                ),
                ViewportEffect::RequestHistoryWindow(query)
                    if query.target_offset_from_bottom == 6
            ));
        }

        #[test]
        fn zero_history_extent_never_starts_a_semantic_viewport_round_trip() {
            let layout = ChromeLayout::new(TerminalSize::new(3, 10), false, ActiveScreen::Main);
            let mut viewport = ViewportController::with_layout(
                layout,
                Some(TerminalScrollMetrics {
                    epoch: Revision::new(1),
                    revision: Revision::new(2),
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 0,
                    viewport_rows: 3,
                }),
            );
            assert!(matches!(viewport.navigate(true, 3), ViewportEffect::None));
            assert!(viewport.is_live());
            let wheel = SgrMouse {
                code: 64,
                column: 10,
                row: 2,
                release: false,
                raw: b"\x1b[<64;10;2M".to_vec(),
            };
            assert!(matches!(
                viewport.handle_gutter_mouse(&wheel, true),
                Some(ViewportEffect::None)
            ));
            assert!(viewport.is_live());
        }

        #[test]
        fn live_sync_fences_new_history_entries_while_pinned_views_keep_reading() {
            let layout = ChromeLayout::new(TerminalSize::new(3, 10), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                offset_from_bottom: 0,
                max_offset_from_bottom: 9,
                viewport_rows: 3,
            };
            assert!(!live_history_navigation_allowed(
                TerminalViewTransportState::Synchronizing
            ));
            assert!(live_history_navigation_allowed(
                TerminalViewTransportState::Active
            ));

            let mut page_up_viewport = ViewportController::with_layout(layout, Some(live));
            let page_up_effect =
                if live_history_navigation_allowed(TerminalViewTransportState::Synchronizing)
                    && history_owns_gestures(ActiveScreen::Main, TerminalModes::default())
                {
                    page_up_viewport.navigate(true, 2)
                } else {
                    ViewportEffect::None
                };
            assert!(matches!(page_up_effect, ViewportEffect::None));
            assert!(page_up_viewport.is_live(), "PageUp stays live during sync");

            let mut main_wheel_viewport = ViewportController::with_layout(layout, Some(live));
            let main_wheel_effect = if main_wheel_viewport.is_live()
                && live_history_navigation_allowed(TerminalViewTransportState::Synchronizing)
                && history_owns_gestures(ActiveScreen::Main, TerminalModes::default())
            {
                main_wheel_viewport.navigate(true, 3)
            } else {
                ViewportEffect::None
            };
            assert!(matches!(main_wheel_effect, ViewportEffect::None));
            assert!(
                main_wheel_viewport.is_live(),
                "main-rectangle wheel stays live during sync"
            );

            let gutter_wheel = SgrMouse {
                code: 64,
                column: 10,
                row: 2,
                release: false,
                raw: b"\x1b[<64;10;2M".to_vec(),
            };
            let gutter_press = SgrMouse {
                code: 0,
                raw: b"\x1b[<0;10;2M".to_vec(),
                ..gutter_wheel.clone()
            };
            let mut live_viewport = ViewportController::with_layout(layout, Some(live));
            assert!(matches!(
                live_viewport.handle_gutter_mouse(&gutter_wheel, false),
                Some(ViewportEffect::None)
            ));
            assert!(matches!(
                live_viewport.handle_gutter_mouse(&gutter_press, false),
                Some(ViewportEffect::None)
            ));
            assert!(live_viewport.is_live());
            assert_eq!(live_viewport.drag_grab_row, None);

            let mut semantic = ViewportController::with_layout(layout, Some(live));
            semantic.window_supported = false;
            assert!(matches!(
                semantic.navigate(true, 3),
                ViewportEffect::RequestViewport(_)
            ));
            semantic
                .apply_viewport(TerminalViewportResult::Frame(TerminalViewportFrame {
                    disposition: zterm_core::terminal::TerminalViewportDisposition::Exact,
                    metrics: TerminalScrollMetrics {
                        offset_from_bottom: 3,
                        ..live
                    },
                    rows: vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()],
                }))
                .expect("semantic history frame");
            assert!(matches!(
                semantic.navigate(true, 3),
                ViewportEffect::RequestViewport(_)
            ));

            let mut legacy = ViewportController::with_layout(layout, None);
            assert!(matches!(
                legacy.navigate(true, 3),
                ViewportEffect::Request(_)
            ));
            legacy
                .apply_history(TerminalHistoryResult::Page(TerminalHistoryPage {
                    cursor: TerminalHistoryCursor {
                        epoch: Revision::new(1),
                        revision: Revision::new(2),
                        start_row: 3,
                        row_count: 3,
                        oldest_row: 0,
                        newest_row: 9,
                    },
                    rows: vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()],
                }))
                .expect("legacy history page");
            assert!(matches!(
                legacy.navigate(true, 3),
                ViewportEffect::Request(HistoryRequest {
                    direction: TerminalHistoryDirection::Older,
                    ..
                })
            ));
        }

        #[test]
        fn live_resume_supersedes_one_inflight_semantic_viewport_response() {
            let layout = ChromeLayout::new(TerminalSize::new(3, 10), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                offset_from_bottom: 0,
                max_offset_from_bottom: 6,
                viewport_rows: 3,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            viewport.window_supported = false;
            assert!(matches!(
                viewport.navigate(true, 3),
                ViewportEffect::RequestViewport(_)
            ));
            assert!(matches!(
                viewport
                    .retain_or_resume(b"key".to_vec())
                    .expect("ordinary input supersedes scrolling"),
                ViewportEffect::Resume
            ));
            viewport.observe_snapshot(Some(live));
            assert_eq!(viewport.finish_resume(), Some(b"key".to_vec()));
            assert!(matches!(
                viewport
                    .apply_viewport(TerminalViewportResult::HistoryGap {
                        epoch: live.epoch,
                        revision: live.revision,
                    })
                    .expect("one superseded response is consumed"),
                ViewportEffect::None
            ));
            assert!(viewport.is_live());
        }

        #[test]
        fn chrome_layout_and_scrollbar_math_cover_threshold_status_and_endpoints() {
            assert_eq!(
                ChromeLayout::new(TerminalSize::new(5, 4), false, ActiveScreen::Main),
                ChromeLayout {
                    child: TerminalSize::new(5, 4),
                    gutter_column: None,
                    status_row: None,
                }
            );
            assert_eq!(
                ChromeLayout::new(TerminalSize::new(5, 5), false, ActiveScreen::Main),
                ChromeLayout {
                    child: TerminalSize::new(5, 4),
                    gutter_column: Some(5),
                    status_row: None,
                }
            );
            assert_eq!(
                ChromeLayout::new(TerminalSize::new(5, 5), true, ActiveScreen::Alternate),
                ChromeLayout {
                    child: TerminalSize::new(4, 5),
                    gutter_column: None,
                    status_row: Some(5),
                }
            );

            let metrics = TerminalScrollMetrics {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                offset_from_bottom: 100,
                max_offset_from_bottom: 100,
                viewport_rows: 20,
            };
            let oldest = ScrollbarGeometry::new(20, metrics).expect("history scrollbar");
            assert_eq!(oldest.thumb_top, 0);
            assert_eq!(oldest.thumb_len, 3);
            let live = ScrollbarGeometry::new(
                20,
                TerminalScrollMetrics {
                    offset_from_bottom: 0,
                    ..metrics
                },
            )
            .expect("live-end scrollbar");
            assert_eq!(live.thumb_top, 17);
            assert_eq!(oldest.offset_for_pointer(1, 1, 100), 100);
            assert_eq!(oldest.offset_for_pointer(20, 1, 100), 0);
            assert!(
                ScrollbarGeometry::new(
                    u16::MAX,
                    TerminalScrollMetrics {
                        max_offset_from_bottom: u64::MAX,
                        offset_from_bottom: u64::MAX / 2,
                        viewport_rows: u16::MAX,
                        ..metrics
                    },
                )
                .is_some()
            );
        }

        #[test]
        fn alternate_snapshot_leaves_reclaimed_rightmost_column_owned_by_child() {
            let physical = TerminalSize::new(3, 8);
            let main_layout = ChromeLayout::new(physical, false, ActiveScreen::Main);
            let mut viewport = ViewportController::with_layout(main_layout, None);
            let mut renderer = TerminalRenderer::new();
            let mut status = StatusRenderer::new(None, physical);
            renderer
                .apply_snapshot_with_chrome(
                    &mut Vec::new(),
                    RenderSnapshot {
                        revision: Revision::new(1),
                        active_screen: ActiveScreen::Main,
                        modes: TerminalModes::default(),
                        recent_history_ansi: b"",
                        screen_ansi: b"\x1b[?1049l\x1b[0m\x1b[2J\x1b[Hmain",
                    },
                    |writer| {
                        write_chrome(
                            writer,
                            &viewport,
                            &mut status,
                            TerminalViewTransportState::Active,
                        )
                    },
                )
                .expect("initial main snapshot");
            viewport.observe_presentation();
            assert_eq!(viewport.presented_gutter_column, Some(8));

            viewport.set_layout(ChromeLayout::new(physical, false, ActiveScreen::Alternate));
            let mut output = ViewportFrameWriter::default();
            let child_frame = "\x1b[0m\x1b[2J\x1b[H\x1b[1;8H▐";
            let mut screen_ansi = b"\x1b[?1049l\x1b[?1049h".to_vec();
            screen_ansi.extend_from_slice(child_frame.as_bytes());
            renderer
                .apply_snapshot_with_chrome(
                    &mut output,
                    RenderSnapshot {
                        revision: Revision::new(2),
                        active_screen: ActiveScreen::Alternate,
                        modes: TerminalModes::default(),
                        recent_history_ansi: b"",
                        screen_ansi: &screen_ansi,
                    },
                    |writer| {
                        write_chrome(
                            writer,
                            &viewport,
                            &mut status,
                            TerminalViewTransportState::Active,
                        )
                    },
                )
                .expect("alternate snapshot transaction");
            viewport.observe_presentation();

            assert_eq!(
                output.bytes,
                [
                    HOST_SYNC_BEGIN,
                    child_frame.as_bytes(),
                    HOST_INPUT_CAPTURE,
                    HOST_SYNC_END,
                ]
                .concat(),
                "Zterm must not clear the reclaimed column after child content"
            );
            assert_eq!(output.frames.len(), 1);
            assert_eq!(output.writes, 1);
            assert_eq!(output.flushes, 1);
            assert_eq!(viewport.presented_gutter_column, None);
        }

        #[test]
        fn main_gutter_relocation_tracks_the_last_presented_column() {
            let initial_layout =
                ChromeLayout::new(TerminalSize::new(3, 8), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                offset_from_bottom: 0,
                max_offset_from_bottom: 6,
                viewport_rows: initial_layout.child.rows,
            };
            let mut viewport = ViewportController::with_layout(initial_layout, Some(live));
            render_scrollbar(&mut Vec::new(), &mut viewport).expect("initial gutter presentation");
            viewport.set_layout(ChromeLayout::new(
                TerminalSize::new(3, 10),
                false,
                ActiveScreen::Main,
            ));
            viewport.set_layout(ChromeLayout::new(
                TerminalSize::new(3, 12),
                false,
                ActiveScreen::Main,
            ));

            let mut output = ViewportFrameWriter::default();
            render_scrollbar(&mut output, &mut viewport).expect("relocated gutter transaction");
            assert_eq!(
                output.bytes,
                [
                    HOST_SYNC_BEGIN,
                    b"\x1b7\x1b[0m\x1b[1;8H \x1b[2;8H \x1b[3;8H \x1b[0m\x1b8".as_slice(),
                    "\x1b7\x1b[0m\x1b[1;12H▕\x1b[2;12H▕\x1b[3;12H▐\x1b[0m\x1b8".as_bytes(),
                    HOST_INPUT_CAPTURE,
                    HOST_SYNC_END,
                ]
                .concat()
            );
            assert_eq!(output.frames.len(), 1);
            assert_eq!(output.writes, 1);
            assert_eq!(output.flushes, 1);
        }

        #[test]
        fn gutter_shrink_repaints_after_any_right_margin_clamp() {
            let initial_layout =
                ChromeLayout::new(TerminalSize::new(3, 10), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                offset_from_bottom: 0,
                max_offset_from_bottom: 6,
                viewport_rows: initial_layout.child.rows,
            };
            let mut viewport = ViewportController::with_layout(initial_layout, Some(live));
            render_scrollbar(&mut Vec::new(), &mut viewport).expect("initial gutter presentation");
            viewport.set_layout(ChromeLayout::new(
                TerminalSize::new(3, 8),
                false,
                ActiveScreen::Main,
            ));

            let mut output = ViewportFrameWriter::default();
            render_scrollbar(&mut output, &mut viewport).expect("narrower gutter transaction");
            assert_eq!(
                output.bytes,
                [
                    HOST_SYNC_BEGIN,
                    b"\x1b7\x1b[0m\x1b[1;10H \x1b[2;10H \x1b[3;10H \x1b[0m\x1b8".as_slice(),
                    "\x1b7\x1b[0m\x1b[1;8H▕\x1b[2;8H▕\x1b[3;8H▐\x1b[0m\x1b8".as_bytes(),
                    HOST_INPUT_CAPTURE,
                    HOST_SYNC_END,
                ]
                .concat(),
                "the new gutter must be written after clearing the out-of-range old column"
            );
            assert_eq!(output.frames.len(), 1);
            assert_eq!(output.writes, 1);
            assert_eq!(output.flushes, 1);
        }

        #[test]
        fn narrow_layout_removes_gutter_without_clamped_cleanup() {
            let initial_layout =
                ChromeLayout::new(TerminalSize::new(3, 5), false, ActiveScreen::Main);
            let mut viewport = ViewportController::with_layout(initial_layout, None);
            render_scrollbar(&mut Vec::new(), &mut viewport).expect("initial gutter presentation");
            viewport.set_layout(ChromeLayout::new(
                TerminalSize::new(3, 4),
                false,
                ActiveScreen::Main,
            ));

            let mut output = ViewportFrameWriter::default();
            render_scrollbar(&mut output, &mut viewport).expect("gutter removal transaction");
            assert_eq!(
                output.bytes,
                [HOST_SYNC_BEGIN, HOST_INPUT_CAPTURE, HOST_SYNC_END].concat(),
                "the clipped gutter column must not be cleared through a clamped coordinate"
            );
            assert_eq!(output.frames.len(), 1);
            assert_eq!(output.writes, 1);
            assert_eq!(output.flushes, 1);
        }

        #[test]
        fn failed_gutter_transaction_does_not_advance_presented_authority() {
            #[derive(Clone, Copy)]
            enum Failure {
                Write,
                Flush,
            }

            struct FailOnceWriter {
                failure: Option<Failure>,
                bytes: Vec<u8>,
            }

            impl Write for FailOnceWriter {
                fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    if matches!(self.failure, Some(Failure::Write)) {
                        self.failure = None;
                        return Err(io::Error::other("injected gutter write failure"));
                    }
                    self.bytes.extend_from_slice(bytes);
                    Ok(bytes.len())
                }

                fn flush(&mut self) -> io::Result<()> {
                    if matches!(self.failure, Some(Failure::Flush)) {
                        self.failure = None;
                        return Err(io::Error::other("injected gutter flush failure"));
                    }
                    Ok(())
                }
            }

            for failure in [Failure::Write, Failure::Flush] {
                let initial_layout =
                    ChromeLayout::new(TerminalSize::new(3, 8), false, ActiveScreen::Main);
                let live = TerminalScrollMetrics {
                    epoch: Revision::new(1),
                    revision: Revision::new(2),
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 6,
                    viewport_rows: initial_layout.child.rows,
                };
                let mut viewport = ViewportController::with_layout(initial_layout, Some(live));
                render_scrollbar(&mut Vec::new(), &mut viewport)
                    .expect("initial gutter presentation");
                assert_eq!(viewport.presented_gutter_column, Some(8));
                viewport.set_layout(ChromeLayout::new(
                    TerminalSize::new(3, 12),
                    false,
                    ActiveScreen::Main,
                ));

                let mut failed = FailOnceWriter {
                    failure: Some(failure),
                    bytes: Vec::new(),
                };
                render_scrollbar(&mut failed, &mut viewport)
                    .expect_err("the injected outer transaction failure must propagate");
                assert_eq!(
                    viewport.presented_gutter_column,
                    Some(8),
                    "building or partially delivering a frame is not presentation authority"
                );

                let mut retry = ViewportFrameWriter::default();
                render_scrollbar(&mut retry, &mut viewport)
                    .expect("retry the complete gutter transaction");
                assert_eq!(
                    retry.bytes,
                    [
                        HOST_SYNC_BEGIN,
                        b"\x1b7\x1b[0m\x1b[1;8H \x1b[2;8H \x1b[3;8H \x1b[0m\x1b8".as_slice(),
                        "\x1b7\x1b[0m\x1b[1;12H▕\x1b[2;12H▕\x1b[3;12H▐\x1b[0m\x1b8".as_bytes(),
                        HOST_INPUT_CAPTURE,
                        HOST_SYNC_END,
                    ]
                    .concat()
                );
                assert_eq!((retry.frames.len(), retry.writes, retry.flushes), (1, 1, 1));
                assert_eq!(viewport.presented_gutter_column, Some(12));
            }
        }

        #[test]
        fn gutter_owns_pointer_before_child_and_renders_redacted_track_transaction() {
            let layout = ChromeLayout::new(TerminalSize::new(4, 8), false, ActiveScreen::Main);
            let metrics = TerminalScrollMetrics {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                offset_from_bottom: 0,
                max_offset_from_bottom: 12,
                viewport_rows: 4,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(metrics));
            let mut output = Vec::new();
            render_scrollbar(&mut output, &mut viewport).expect("render scrollbar transaction");
            assert!(contains_bytes(&output, "▕".as_bytes()));
            assert!(contains_bytes(&output, "▐".as_bytes()));
            assert!(output.ends_with(&[HOST_INPUT_CAPTURE, HOST_SYNC_END].concat()));

            let gutter_press = SgrMouse {
                code: 0,
                column: 8,
                row: 1,
                release: false,
                raw: b"\x1b[<0;8;1M".to_vec(),
            };
            assert!(matches!(
                viewport.handle_gutter_mouse(&gutter_press, true),
                Some(ViewportEffect::RequestHistoryWindow(query))
                    if query.target_offset_from_bottom == 12
            ));
            let child_press = SgrMouse {
                column: 7,
                raw: b"\x1b[<0;7;1M".to_vec(),
                ..gutter_press
            };
            assert!(viewport.handle_gutter_mouse(&child_press, true).is_none());
        }

        #[test]
        fn active_scrollbar_drag_owns_motion_and_release_outside_the_gutter() {
            let layout = ChromeLayout::new(TerminalSize::new(4, 8), false, ActiveScreen::Main);
            let metrics = TerminalScrollMetrics {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                offset_from_bottom: 0,
                max_offset_from_bottom: 12,
                viewport_rows: 4,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(metrics));
            let press = SgrMouse {
                code: 0,
                column: 8,
                row: 1,
                release: false,
                raw: b"\x1b[<0;8;1M".to_vec(),
            };
            assert!(matches!(
                viewport.handle_gutter_mouse(&press, true),
                Some(ViewportEffect::RequestHistoryWindow(query))
                    if query.target_offset_from_bottom == 12
            ));
            assert!(viewport.drag_grab_row.is_some());

            let outside_motion = SgrMouse {
                code: 32,
                column: 2,
                row: u16::MAX,
                release: false,
                raw: b"\x1b[<32;2;65535M".to_vec(),
            };
            assert!(matches!(
                viewport.handle_gutter_mouse(&outside_motion, true),
                Some(ViewportEffect::None)
            ));
            assert!(viewport.drag_grab_row.is_some());

            let outside_release = SgrMouse {
                code: 0,
                column: 2,
                row: u16::MAX,
                release: true,
                raw: b"\x1b[<0;2;65535m".to_vec(),
            };
            assert!(matches!(
                viewport.handle_gutter_mouse(&outside_release, true),
                Some(ViewportEffect::Resume)
            ));
            assert_eq!(viewport.drag_grab_row, None);
            assert!(
                viewport
                    .handle_gutter_mouse(&outside_motion, true)
                    .is_none()
            );
        }

        #[test]
        fn legacy_history_keeps_the_reserved_gutter_blank_without_a_fake_thumb() {
            let layout = ChromeLayout::new(TerminalSize::new(3, 8), false, ActiveScreen::Main);
            let mut viewport = ViewportController::with_layout(layout, None);
            assert!(matches!(
                viewport.navigate(true, 3),
                ViewportEffect::Request(HistoryRequest {
                    direction: TerminalHistoryDirection::Newest,
                    cursor: None,
                })
            ));
            viewport
                .apply_history(TerminalHistoryResult::Page(TerminalHistoryPage {
                    cursor: TerminalHistoryCursor {
                        epoch: Revision::new(1),
                        revision: Revision::new(2),
                        start_row: 0,
                        row_count: 3,
                        oldest_row: 0,
                        newest_row: 9,
                    },
                    rows: vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()],
                }))
                .expect("legacy page");
            assert_eq!(viewport.scroll_metrics(), None);
            let mut output = Vec::new();
            write_scrollbar(&mut output, &viewport).expect("blank legacy gutter");
            assert!(!contains_bytes(&output, "▕".as_bytes()));
            assert!(!contains_bytes(&output, "▐".as_bytes()));
        }

        #[test]
        fn background_resync_sequence_preserves_history_but_reconnect_resets_presentation() {
            let layout = ChromeLayout::new(TerminalSize::new(3, 8), false, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                offset_from_bottom: 0,
                max_offset_from_bottom: 9,
                viewport_rows: 3,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            viewport.window_supported = false;
            assert!(matches!(
                viewport.navigate(true, 3),
                ViewportEffect::RequestViewport(_)
            ));
            viewport
                .apply_viewport(TerminalViewportResult::Frame(TerminalViewportFrame {
                    disposition: zterm_core::terminal::TerminalViewportDisposition::Exact,
                    metrics: TerminalScrollMetrics {
                        offset_from_bottom: 3,
                        ..live
                    },
                    rows: vec![
                        b"fixed-a".to_vec(),
                        b"fixed-b".to_vec(),
                        b"fixed-c".to_vec(),
                    ],
                }))
                .expect("semantic history frame");
            let fixed = viewport
                .visible_history_rows()
                .expect("pinned history")
                .0
                .to_vec();

            let mut renderer = TerminalRenderer::new();
            renderer
                .apply_snapshot(
                    &mut Vec::new(),
                    RenderSnapshot {
                        revision: Revision::new(2),
                        active_screen: ActiveScreen::Main,
                        modes: TerminalModes::default(),
                        recent_history_ansi: b"",
                        screen_ansi: b"\x1b[?1049llive",
                    },
                )
                .expect("initial main snapshot");
            let background_metrics = TerminalScrollMetrics {
                revision: Revision::new(3),
                ..live
            };
            viewport.observe_sync_required();
            assert!(
                viewport.is_history(),
                "the in-flight resync marker stays pinned"
            );
            viewport.observe_snapshot(Some(background_metrics));
            renderer
                .observe_snapshot(RenderSnapshot {
                    revision: Revision::new(3),
                    active_screen: ActiveScreen::Alternate,
                    modes: TerminalModes::default(),
                    recent_history_ansi: b"",
                    screen_ansi: b"\x1b[?1049l\x1b[?1049hfullscreen",
                })
                .expect("background alternate snapshot validates without repainting it");
            assert!(viewport.is_history());
            assert_eq!(
                viewport.visible_history_rows().expect("still pinned").0,
                fixed
            );
            assert_eq!(
                viewport.effective_screen(renderer.active_screen()),
                ActiveScreen::Main
            );

            viewport.drag_grab_row = Some(0);
            viewport.reset_presentation_for_reconnect();
            assert!(viewport.is_resume_pending());
            assert_eq!(viewport.drag_grab_row, None);
            assert_eq!(viewport.scroll_metrics(), None);
            viewport.observe_snapshot(Some(background_metrics));
            assert_eq!(viewport.finish_resume(), Some(Vec::new()));
            assert!(viewport.is_live());

            viewport.drag_grab_row = Some(1);
            viewport.reset_presentation_for_reconnect();
            assert_eq!(
                viewport.drag_grab_row, None,
                "transport capture loss also clears a live drag"
            );
            assert_eq!(
                viewport.scroll_metrics(),
                None,
                "a new attachment epoch must not reuse live scrollbar identity"
            );
        }

        #[test]
        fn snapshot_history_precedes_screen_and_deltas_require_contiguity() {
            let mut renderer = TerminalRenderer::new();
            let mut output = Vec::new();
            renderer
                .apply_snapshot(
                    &mut output,
                    RenderSnapshot {
                        revision: Revision::new(4),
                        active_screen: ActiveScreen::Main,
                        modes: TerminalModes::default(),
                        recent_history_ansi: b"history",
                        screen_ansi: b"\x1b[?1049lscreen",
                    },
                )
                .expect("snapshot");
            assert_eq!(
                output,
                [
                    HOST_SYNC_BEGIN,
                    b"historyscreen".as_slice(),
                    HOST_INPUT_CAPTURE,
                    HOST_SYNC_END,
                ]
                .concat()
            );

            assert_eq!(
                renderer
                    .apply_delta(
                        &mut output,
                        RenderDelta {
                            from_revision: Revision::new(3),
                            to_revision: Revision::new(5),
                            active_screen: ActiveScreen::Main,
                            modes: TerminalModes::default(),
                            ansi: b"stale",
                        },
                    )
                    .expect("gap"),
                DeltaRender::Gap
            );
            assert_eq!(
                output,
                [
                    HOST_SYNC_BEGIN,
                    b"historyscreen".as_slice(),
                    HOST_INPUT_CAPTURE,
                    HOST_SYNC_END,
                ]
                .concat()
            );

            assert_eq!(
                renderer
                    .apply_delta(
                        &mut output,
                        RenderDelta {
                            from_revision: Revision::new(4),
                            to_revision: Revision::new(5),
                            active_screen: ActiveScreen::Alternate,
                            modes: TerminalModes::default(),
                            ansi: b"\x1b[?1049hdelta",
                        },
                    )
                    .expect("delta"),
                DeltaRender::Applied
            );
            assert_eq!(
                output,
                [
                    HOST_SYNC_BEGIN,
                    b"historyscreen".as_slice(),
                    HOST_INPUT_CAPTURE,
                    HOST_SYNC_END,
                    HOST_SYNC_BEGIN,
                    b"delta",
                    HOST_INPUT_CAPTURE,
                    HOST_SYNC_END,
                ]
                .concat()
            );
        }

        #[test]
        fn renderer_reasserts_ui_mouse_capture_after_child_disables_matching_modes() {
            let child_capture = TerminalModes {
                mouse_mode: TerminalMouseMode::AnyMotion,
                mouse_encoding: TerminalMouseEncoding::Sgr,
                ..TerminalModes::default()
            };
            let mut renderer = TerminalRenderer::new();
            renderer
                .apply_snapshot(
                    &mut Vec::new(),
                    RenderSnapshot {
                        revision: Revision::new(1),
                        active_screen: ActiveScreen::Main,
                        modes: child_capture,
                        recent_history_ansi: b"",
                        screen_ansi: b"\x1b[?1049l\x1b[?1003h\x1b[?1006h",
                    },
                )
                .expect("establish child mouse modes");

            let child_disable = b"\x1b[?1003l\x1b[?1006l";
            let mut output = Vec::new();
            assert_eq!(
                renderer
                    .apply_delta(
                        &mut output,
                        RenderDelta {
                            from_revision: Revision::new(1),
                            to_revision: Revision::new(2),
                            active_screen: ActiveScreen::Main,
                            modes: TerminalModes::default(),
                            ansi: child_disable,
                        },
                    )
                    .expect("apply child mouse disable"),
                DeltaRender::Applied
            );
            assert_eq!(
                output,
                [
                    HOST_SYNC_BEGIN,
                    child_disable.as_slice(),
                    HOST_INPUT_CAPTURE,
                    HOST_SYNC_END,
                ]
                .concat()
            );
            assert_eq!(renderer.modes(), TerminalModes::default());
            assert!(history_owns_gestures(
                renderer.active_screen(),
                renderer.modes()
            ));
        }

        #[test]
        fn renderer_virtualizes_every_terminal_model_screen_selector() {
            assert!(
                snapshot_screen_ansi(b"\x1b[?1049l\x1b[?1049hscreen", ActiveScreen::Main).is_err(),
                "main snapshots cannot leak an alternate-screen selector"
            );
            assert!(
                delta_screen_ansi(b"delta", ActiveScreen::Main, ActiveScreen::Alternate).is_err(),
                "screen-changing deltas require the matching model selector"
            );
            assert!(
                delta_screen_ansi(
                    b"\x1b[?1049hdelta\x1b[?1049l",
                    ActiveScreen::Main,
                    ActiveScreen::Alternate,
                )
                .is_err(),
                "a second selector cannot escape zterm's outer alternate screen"
            );
        }

        #[test]
        fn child_chrome_capture_render_transactions_have_exact_bytes_and_one_flush() {
            #[derive(Default)]
            struct RecordingWriter {
                bytes: Vec<u8>,
                writes: usize,
                flushes: usize,
            }

            impl Write for RecordingWriter {
                fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    self.writes += 1;
                    self.bytes.extend_from_slice(bytes);
                    Ok(bytes.len())
                }

                fn flush(&mut self) -> io::Result<()> {
                    self.flushes += 1;
                    Ok(())
                }
            }

            let layout = ChromeLayout::new(TerminalSize::new(3, 8), true, ActiveScreen::Main);
            let live = TerminalScrollMetrics {
                epoch: Revision::new(2),
                revision: Revision::new(9),
                offset_from_bottom: 0,
                max_offset_from_bottom: 6,
                viewport_rows: layout.child.rows,
            };
            let mut viewport = ViewportController::with_layout(layout, Some(live));
            let mut status = StatusRenderer::new(Some("dev".to_owned()), TerminalSize::new(3, 8));
            status.path = TerminalViewConnectionPath::Direct;
            status.rtt_ms = Some(4);
            let mut expected_status =
                StatusRenderer::new(Some("dev".to_owned()), TerminalSize::new(3, 8));
            expected_status.path = TerminalViewConnectionPath::Direct;
            expected_status.rtt_ms = Some(4);

            let mut renderer = TerminalRenderer::new();
            let mut output = RecordingWriter::default();
            renderer
                .apply_snapshot_with_chrome(
                    &mut output,
                    RenderSnapshot {
                        revision: Revision::new(9),
                        active_screen: ActiveScreen::Main,
                        modes: TerminalModes::default(),
                        recent_history_ansi: b"history",
                        screen_ansi: b"\x1b[?1049lscreen",
                    },
                    |writer| {
                        write_chrome(
                            writer,
                            &viewport,
                            &mut status,
                            TerminalViewTransportState::Active,
                        )
                    },
                )
                .expect("fully flushed snapshot");
            viewport.observe_presentation();
            let mut expected = HOST_SYNC_BEGIN.to_vec();
            expected.extend_from_slice(b"historyscreen");
            write_chrome(
                &mut expected,
                &viewport,
                &mut expected_status,
                TerminalViewTransportState::Active,
            )
            .expect("expected snapshot chrome");
            expected.extend_from_slice(HOST_INPUT_CAPTURE);
            expected.extend_from_slice(HOST_SYNC_END);
            assert_eq!(output.bytes, expected);
            assert_eq!(output.writes, 1);
            assert_eq!(output.flushes, 1);
            assert_eq!(renderer.revision(), Revision::new(9));

            renderer
                .apply_delta_with_chrome(
                    &mut output,
                    RenderDelta {
                        from_revision: Revision::new(9),
                        to_revision: Revision::new(10),
                        active_screen: ActiveScreen::Main,
                        modes: TerminalModes::default(),
                        ansi: b"delta",
                    },
                    |writer| {
                        write_chrome(
                            writer,
                            &viewport,
                            &mut status,
                            TerminalViewTransportState::Active,
                        )
                    },
                )
                .expect("fully flushed delta");
            viewport.observe_presentation();
            expected.extend_from_slice(HOST_SYNC_BEGIN);
            expected.extend_from_slice(b"delta");
            write_chrome(
                &mut expected,
                &viewport,
                &mut expected_status,
                TerminalViewTransportState::Active,
            )
            .expect("expected delta chrome");
            expected.extend_from_slice(HOST_INPUT_CAPTURE);
            expected.extend_from_slice(HOST_SYNC_END);
            assert_eq!(output.bytes, expected);
            assert_eq!(output.writes, 2);
            assert_eq!(output.flushes, 2);

            viewport.window_supported = false;
            assert!(matches!(
                viewport.navigate(true, 3),
                ViewportEffect::RequestViewport(_)
            ));
            viewport
                .apply_viewport(TerminalViewportResult::Frame(TerminalViewportFrame {
                    disposition: zterm_core::terminal::TerminalViewportDisposition::Exact,
                    metrics: TerminalScrollMetrics {
                        offset_from_bottom: 3,
                        ..live
                    },
                    rows: vec![b"older".to_vec(), b"newer".to_vec()],
                }))
                .expect("history frame");
            render_history_with_chrome(
                &mut output,
                &viewport,
                &mut status,
                TerminalViewTransportState::Active,
            )
            .expect("fully flushed history and chrome");
            viewport.observe_presentation();
            expected.extend_from_slice(HOST_SYNC_BEGIN);
            write_history(&mut expected, &viewport).expect("expected history rows");
            write_chrome(
                &mut expected,
                &viewport,
                &mut expected_status,
                TerminalViewTransportState::Active,
            )
            .expect("expected history chrome");
            expected.extend_from_slice(HOST_INPUT_CAPTURE);
            expected.extend_from_slice(HOST_SYNC_END);
            assert_eq!(output.bytes, expected);
            assert_eq!(output.writes, 3);
            assert_eq!(output.flushes, 3);
        }

        #[test]
        fn partial_atomic_write_best_effort_closes_synchronized_presentation() {
            #[derive(Default)]
            struct PartialWriter {
                bytes: Vec<u8>,
                writes: usize,
                flushes: usize,
            }

            impl Write for PartialWriter {
                fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    self.writes += 1;
                    if self.writes == 2 {
                        return Err(io::Error::other("injected partial write failure"));
                    }
                    let count = if self.writes == 1 {
                        bytes.len().min(4)
                    } else {
                        bytes.len()
                    };
                    self.bytes.extend_from_slice(&bytes[..count]);
                    Ok(count)
                }

                fn flush(&mut self) -> io::Result<()> {
                    self.flushes += 1;
                    Ok(())
                }
            }

            let mut writer = PartialWriter::default();
            present_atomic(&mut writer, "test atomic presentation", |frame| {
                frame.write_all(b"replacement")
            })
            .expect_err("the injected partial write remains visible to the caller");

            assert!(writer.bytes.ends_with(HOST_SYNC_END));
            assert_eq!(writer.writes, 3);
            assert_eq!(writer.flushes, 1);
        }

        #[test]
        fn reconnect_status_is_fixed_flushed_and_only_emitted_while_reconnecting() {
            #[derive(Default)]
            struct RecordingWriter {
                bytes: Vec<u8>,
                flushes: usize,
            }

            impl Write for RecordingWriter {
                fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    self.bytes.extend_from_slice(bytes);
                    Ok(bytes.len())
                }

                fn flush(&mut self) -> io::Result<()> {
                    self.flushes += 1;
                    Ok(())
                }
            }

            let mut output = RecordingWriter::default();
            render_transport_state(&mut output, TerminalViewTransportState::Synchronizing)
                .expect("synchronizing is silent");
            render_transport_state(&mut output, TerminalViewTransportState::Reconnecting)
                .expect("reconnecting status");
            render_transport_state(&mut output, TerminalViewTransportState::Active)
                .expect("active is silent");
            assert_eq!(
                output.bytes,
                [
                    HOST_SYNC_BEGIN,
                    RECONNECTING_STATUS,
                    HOST_INPUT_CAPTURE,
                    HOST_SYNC_END,
                ]
                .concat()
            );
            assert_eq!(output.flushes, 1);
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
