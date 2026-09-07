//! Application-owned native terminal reducer. UI observers only read complete frames.
mod navigation;
use navigation::Navigation;

use crate::{
    NativeError, NativeRuntime,
    network::{parse_device, require_network},
};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use zterm_client::{
    keyboard::{FunctionalKey, KeyCode, KeyEventKind, KeyInput},
    model::ResolvedSessionTarget,
    session::SessionClient,
    surface::AttachmentSurface,
    view::{
        PreparedTerminalView, TerminalViewCommandWriter, TerminalViewEvent, TerminalViewRoute,
        TerminalViewTarget, TerminalViewTransportState,
    },
};
use zterm_core::{ResourceLimits, SessionId, SessionSelector, terminal::*};

/// Measured grid; pixels, density and IME geometry stay in the Android View.
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct NativeViewport {
    /// Complete visible rows.
    pub rows: u16,
    /// Complete visible columns.
    pub columns: u16,
}
impl NativeViewport {
    pub(crate) fn size(self) -> Result<TerminalSize, NativeError> {
        let limits = ResourceLimits::default();
        if self.rows == 0
            || self.columns == 0
            || self.rows > limits.max_viewport_rows
            || self.columns > limits.max_viewport_columns
        {
            return Err(failure("invalid_viewport"));
        }
        Ok(TerminalSize::new(self.rows, self.columns))
    }
}
/// Complete display cell. Text never participates in Debug/log output.
#[derive(Clone, uniffi::Record)]
pub struct NativeCell {
    /// Exact cluster, including combining scalars.
    pub text: String,
    /// Column width: zero for a wide continuation.
    pub width: u8,
    /// Resolved opaque ARGB foreground.
    pub foreground: u32,
    /// Resolved opaque ARGB background.
    pub background: u32,
    /// Bold=1, dim=2, italic=4.
    pub attributes: u8,
    /// Underline kind: none/single/double/curly/dotted/dashed.
    pub underline: u8,
    /// Resolved underline color.
    pub underline_color: u32,
}
/// One complete row, never a per-cell foreign callback.
#[derive(Clone, uniffi::Record)]
pub struct NativeRow {
    /// Exact rectangular cells.
    pub cells: Vec<NativeCell>,
    /// Soft-wrap continuity.
    pub wrapped: bool,
}
/// Latest complete frame, replacing any unobserved predecessor.
#[derive(Clone, uniffi::Record)]
pub struct NativeFrame {
    /// Gesture ownership of the synchronized live grid, never of pinned history.
    pub pointer_mode: NativePointerMode,
    /// Opaque accounted semantic source for the exact frame drawn by Android.
    pub source: Option<Arc<NativeFrameSource>>,
    /// Local, content-free resource and query counters for acceptance evidence.
    pub stats: NativeNavigationStats,
    /// Recoverable local reading failure; independent of transport health.
    pub notice: Option<String>,
    /// Input epoch changes only when synchronized admission becomes invalid.
    pub input_epoch: u64,
    /// Same-attachment return-to-bottom may admit bounded input before its ACK.
    pub input_ready: bool,
    /// Local reading offset; no separate user-visible history mode.
    pub history_offset: u64,
    /// Logical ordinal of the first drawn row for selection hit testing.
    pub first_row: i64,
    /// Frozen selection endpoints in logical content coordinates.
    pub selection: Option<NativeSelection>,
    /// Monotonic local display version, independent of wire revision.
    pub generation: u64,
    /// State code: synchronizing/active/reconnecting/ended/lease_lost/closed.
    pub state: String,
    /// Stable domain failure code, never a remote diagnostic.
    pub error: Option<String>,
    /// Exact dimensions.
    pub viewport: NativeViewport,
    /// Complete presentation.
    pub rows: Vec<NativeRow>,
    /// IME anchor row, retained even when cursor is hidden.
    pub cursor_row: u16,
    /// IME anchor column.
    pub cursor_column: u16,
    /// Whether to draw the cursor glyph.
    pub cursor_visible: bool,
    /// Resolved cursor block color.
    pub cursor_color: u32,
    /// Default canvas color.
    pub background: u32,
    /// Available retained-history offset.
    pub history_maximum: u64,
}
/// Content-free mobile navigation evidence. Never contains addresses or text.
#[derive(Clone, Default, uniffi::Record)]
pub struct NativeNavigationStats {
    /// Actual shared-driver history requests, including prefetch.
    pub requests: u64,
    /// Gesture targets rendered from already retained pages.
    pub cache_hits: u64,
    /// Gesture targets requiring a page response.
    pub cache_misses: u64,
    /// Accounted rows, including externally pinned captured sources.
    pub retained_rows: u32,
    /// Accounted semantic allocation and reserved selection metadata bytes.
    pub retained_bytes: u64,
    /// Maximum retained allocation for this attachment.
    pub peak_bytes: u64,
    /// Smoothed observed history response latency.
    pub query_rtt_ms: u64,
}
/// Immutable semantic source retained only while a frame is observed or selected.
#[derive(Clone, uniffi::Object)]
pub struct NativeFrameSource {
    page: Arc<zterm_core::viewport_cache::CachedViewportWindow<TerminalSurfaceRow>>,
    offset: u64,
    input_epoch: u64,
    screen: ActiveScreen,
    origin: Arc<()>,
    modes: TerminalModes,
    pointer_mode: NativePointerMode,
}
/// Child-declared touch behavior, separate from native long-press selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum NativePointerMode {
    /// Local viewport owns touch scrolling.
    None,
    /// Tap/click and wheel reports go to the child.
    Mouse,
    /// A wheel step becomes one cursor key on the alternate screen.
    AlternateScroll,
}
impl NativePointerMode {
    fn for_surface(surface: &AttachmentSurface) -> Self {
        if surface.modes().mouse_mode != TerminalMouseMode::None {
            Self::Mouse
        } else if surface.active_screen() == ActiveScreen::Alternate
            && surface.modes().alternate_scroll
        {
            Self::AlternateScroll
        } else {
            Self::None
        }
    }
}
#[uniffi::export]
impl NativeFrameSource {
    /// Gives the View an independent handle while the repository replaces its frame.
    pub fn retained(&self) -> Arc<Self> {
        Arc::new(self.clone())
    }
}

/// Selection coordinates; the Android View owns handles and floating actions.
#[derive(Clone, uniffi::Record)]
pub struct NativeSelection {
    /// Logical anchor row (may be off screen).
    pub anchor_row: i64,
    /// Anchor column.
    pub anchor_column: u16,
    /// Logical moving endpoint row.
    pub focus_row: i64,
    /// Moving endpoint column.
    pub focus_column: u16,
}
/// Typed physical/shortcut key; no platform ANSI parser crosses this boundary.
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum NativeKey {
    /// Escape.
    Escape,
    /// Enter.
    Enter,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Left.
    Left,
    /// Down.
    Down,
    /// Up.
    Up,
    /// Right.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Function key, one through twelve.
    Function {
        /// Key number.
        number: u8,
    },
    /// Printable physical identity.
    Unicode {
        /// Unshifted Unicode scalar.
        scalar: u32,
    },
}
impl NativeKey {
    fn code(self) -> KeyCode {
        let key = match self {
            Self::Unicode { scalar } => return KeyCode::Unicode(scalar),
            Self::Escape => FunctionalKey::Escape,
            Self::Enter => FunctionalKey::Enter,
            Self::Tab => FunctionalKey::Tab,
            Self::Backspace => FunctionalKey::Backspace,
            Self::Delete => FunctionalKey::Delete,
            Self::Left => FunctionalKey::Left,
            Self::Down => FunctionalKey::Down,
            Self::Up => FunctionalKey::Up,
            Self::Right => FunctionalKey::Right,
            Self::Home => FunctionalKey::Home,
            Self::End => FunctionalKey::End,
            Self::PageUp => FunctionalKey::PageUp,
            Self::PageDown => FunctionalKey::PageDown,
            Self::Function { number } => FunctionalKey::Function(number),
        };
        KeyCode::Functional(key)
    }
}
/// One attachment retained by the Application, not by an Activity's subscription.
#[derive(uniffi::Object)]
pub struct NativeTerminal {
    commands: mpsc::Sender<Command>,
    frame: watch::Sender<NativeFrame>,
    cancel: CancellationToken,
}
enum Action {
    Pointer {
        source: Arc<NativeFrameSource>,
        row: u16,
        column: u16,
        wheel_lines: i16,
    },
    Text {
        text: String,
        modifiers: u8,
        paste: bool,
    },
    Key {
        key: NativeKey,
        modifiers: u8,
        kind: u8,
        text: String,
    },
    Scroll {
        offset: u64,
        older: bool,
        horizon: u8,
    },
    Select {
        source: Arc<NativeFrameSource>,
        row: u16,
        column: u16,
    },
    Extend {
        source: Arc<NativeFrameSource>,
        row: u16,
        column: u16,
        anchor: bool,
    },
    ClearSelection,
    ClearNotice,
    Copy(oneshot::Sender<Result<String, NativeError>>),
    Visible(bool),
    Resize(NativeViewport),
    Colors(bool),
    Detach,
}
struct Command {
    input_epoch: u64,
    action: Action,
    reply: oneshot::Sender<Result<(), NativeError>>,
}
#[uniffi::export]
impl NativeTerminal {
    /// Complete tap (zero) or bounded signed wheel steps (positive = up).
    /// Coordinates are zero-based in the actually drawn source; never replayed.
    pub async fn send_pointer(
        &self,
        source: Arc<NativeFrameSource>,
        row: u16,
        column: u16,
        wheel_lines: i16,
    ) -> Result<(), NativeError> {
        if wheel_lines.unsigned_abs() > 32 {
            return Err(failure("invalid_input"));
        }
        self.submit_at(
            source.input_epoch,
            Action::Pointer {
                source,
                row,
                column,
                wheel_lines,
            },
        )
        .await
    }
    /// Reads the complete current frame without starting another producer.
    pub fn current_frame(&self) -> NativeFrame {
        self.frame.borrow().clone()
    }
    /// Waits for replacement; cancelling the waiter leaves the attachment running.
    pub async fn wait_for_frame(&self, after_generation: u64) -> Result<NativeFrame, NativeError> {
        let mut receiver = self.frame.subscribe();
        loop {
            let frame = receiver.borrow_and_update().clone();
            if frame.generation > after_generation {
                return Ok(frame);
            }
            tokio::select! { biased; _ = self.cancel.cancelled() => {
                // The actor publishes its final state before cancellation. A
                // waiter already asleep must observe that frame even when both
                // signals become ready together (lease loss / Session end).
                let final_frame = receiver.borrow_and_update().clone();
                return if final_frame.generation > after_generation {
                    Ok(final_frame)
                } else { Err(NativeError::Closed) };
            },
            changed = receiver.changed() => { changed.map_err(|_| NativeError::Closed)?; } }
        }
    }
    /// Sends committed IME text once, with optional one-shot modifiers.
    pub async fn commit_text(
        &self,
        input_epoch: u64,
        text: String,
        modifiers: u8,
        paste: bool,
    ) -> Result<(), NativeError> {
        if text.len() > zterm_client::input::RESUME_INPUT_BOUND
            || (paste && text.len() > MAX_TERMINAL_CLIPBOARD_BYTES)
        {
            return Err(failure("resource_limit"));
        }
        self.submit_at(
            input_epoch,
            Action::Text {
                text,
                modifiers,
                paste,
            },
        )
        .await
    }
    /// Sends one physical or shortcut event. Kind 1=press, 2=repeat, 3=release.
    pub async fn send_key(
        &self,
        input_epoch: u64,
        key: NativeKey,
        modifiers: u8,
        kind: u8,
        text: String,
    ) -> Result<(), NativeError> {
        if text.len() > 128 || !(1..=3).contains(&kind) {
            return Err(failure("invalid_input"));
        }
        self.submit_at(
            input_epoch,
            Action::Key {
                key,
                modifiers,
                kind,
                text,
            },
        )
        .await
    }
    /// Local scroll intent; the shared cache decides whether any network read is needed.
    pub async fn scroll_to(
        &self,
        offset: u64,
        older: bool,
        horizon: u8,
    ) -> Result<(), NativeError> {
        self.submit(Action::Scroll {
            offset,
            older,
            horizon,
        })
        .await
    }
    /// Captures exactly the complete frame hit by the user's long press.
    pub async fn begin_selection(
        &self,
        source: Arc<NativeFrameSource>,
        row: u16,
        column: u16,
    ) -> Result<(), NativeError> {
        self.submit(Action::Select {
            source,
            row,
            column,
        })
        .await
    }
    /// Moves one native selection handle in the displayed frame.
    pub async fn extend_selection(
        &self,
        source: Arc<NativeFrameSource>,
        row: u16,
        column: u16,
        anchor: bool,
    ) -> Result<(), NativeError> {
        self.submit(Action::Extend {
            source,
            row,
            column,
            anchor,
        })
        .await
    }
    /// Cancels local selection without sending host input.
    pub async fn clear_selection(&self) -> Result<(), NativeError> {
        self.submit(Action::ClearSelection).await
    }
    /// Dismisses a local reading notice without changing its captured selection.
    pub async fn clear_notice(&self) -> Result<(), NativeError> {
        self.submit(Action::ClearNotice).await
    }
    /// Returns one atomically validated local copy; never sends Ctrl+C.
    pub async fn copy_selection(&self) -> Result<String, NativeError> {
        let (send, result) = oneshot::channel();
        self.submit(Action::Copy(send)).await?;
        result.await.map_err(|_| NativeError::Closed)?
    }
    /// Pauses speculative history work only; the connection stays owned by the App.
    pub async fn set_visible(&self, visible: bool) -> Result<(), NativeError> {
        self.submit(Action::Visible(visible)).await
    }

    /// Coalesced measured size from the native View.
    pub async fn resize(&self, viewport: NativeViewport) -> Result<(), NativeError> {
        viewport.size()?;
        self.submit(Action::Resize(viewport)).await
    }
    /// Changes default terminal observations without replacing its Session.
    pub async fn set_dark(&self, dark: bool) -> Result<(), NativeError> {
        self.submit(Action::Colors(dark)).await
    }
    /// Explicit navigation detach leaves the host process alive.
    pub async fn detach(&self) -> Result<(), NativeError> {
        self.submit(Action::Detach).await
    }
}
impl NativeTerminal {
    async fn submit(&self, action: Action) -> Result<(), NativeError> {
        let epoch = self.frame.borrow().input_epoch;
        self.submit_at(epoch, action).await
    }
    async fn submit_at(&self, input_epoch: u64, action: Action) -> Result<(), NativeError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .try_send(Command {
                input_epoch,
                action,
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => failure("resource_limit"),
                mpsc::error::TrySendError::Closed(_) => NativeError::Closed,
            })?;
        result.await.map_err(|_| NativeError::Closed)?
    }
}
impl Drop for NativeTerminal {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[uniffi::export]
impl NativeRuntime {
    /// Attaches only to the supplied exact Session. No implicit create or retarget.
    pub async fn connect_terminal(
        &self,
        host: String,
        session: String,
        viewport: NativeViewport,
        dark: bool,
        takeover: bool,
    ) -> Result<Arc<NativeTerminal>, NativeError> {
        let host = parse_device(&host)?;
        let session = parse_session(&session)?;
        let size = viewport.size()?;
        let network = Arc::clone(&self.network);
        let closed = self.closed.clone();
        self.on_executor(async move {
            let controller = require_network(&network)?.controller.clone();
            let client = SessionClient::connect(
                Arc::new(controller),
                ResolvedSessionTarget::device(host),
                Some(SessionSelector::Id(session)),
                false,
                takeover,
                Some(size),
                base_colors(dark),
            )
            .await?;
            let prepared = PreparedTerminalView::new(
                client,
                takeover,
                TerminalViewTarget::for_display("", TerminalViewRoute::Remote),
            )?;
            let surface = AttachmentSurface::from_snapshot(prepared.initial_snapshot())
                .map_err(|_| failure("malformed_frame"))?;
            let (frame, _) = watch::channel(project(
                &surface,
                1,
                "synchronizing",
                None,
                dark,
                &surface.surface.rows,
            ));
            // The complete surface is atomically installed before its exact ACK.
            let io = prepared.acknowledge_initial().await?;
            let (reader, writer) = io.split();
            let (commands, receiver) = mpsc::channel(32);
            let cancel = closed.child_token();
            let terminal = Arc::new(NativeTerminal {
                commands,
                frame: frame.clone(),
                cancel: cancel.clone(),
            });
            tokio::spawn(run(surface, reader, writer, receiver, frame, cancel, dark));
            // Attach's viewport is only a creation hint on the host. A retained
            // Session still has its previous controller's dimensions. Install
            // the requested size through the actor before exposing this handle;
            // the actor defers the resize until takeover/Active admits it.
            terminal.resize(viewport).await?;
            Ok(terminal)
        })
        .await
    }
}
async fn run(
    mut surface: AttachmentSurface,
    mut reader: zterm_client::view::TerminalViewEventReader,
    writer: TerminalViewCommandWriter,
    mut commands: mpsc::Receiver<Command>,
    frame: watch::Sender<NativeFrame>,
    cancel: CancellationToken,
    mut dark: bool,
) {
    let mut state = "synchronizing";
    let mut generation = 1;
    let mut input_epoch = 1;
    let mut desired_size = surface.surface.size;
    let mut navigation = Navigation::new(&surface);
    let origin = Arc::new(());
    loop {
        let mut query = None;
        let mut prefetch = false;
        let result: Result<(), NativeError> = tokio::select! {
            _ = cancel.cancelled() => { let _ = writer.detach().await; break; }
            command = commands.recv() => {
                let Some(command) = command else { let _ = writer.detach().await; break; };
                let detach = matches!(command.action, Action::Detach);
                let result: Result<(), NativeError> = match command.action {
                    Action::Detach => writer.detach().await.map_err(Into::into),
                    Action::Copy(reply) => { let _ = reply.send(navigation.copy().map(TerminalClipboardWrite::into_string)); Ok(()) },
                    Action::ClearSelection => { navigation.clear_selection(); Ok(()) },
                    Action::ClearNotice => { navigation.notice = None; Ok(()) },
                    Action::Visible(value) => { navigation.visible = value; prefetch = value; Ok(()) },
                    Action::Resize(viewport) => match viewport.size() {
                        Ok(size) => { desired_size = size;
                            if size != surface.surface.size {
                                navigation.disconnected(); input_epoch += 1;
                                if state == "active" { state = "synchronizing"; writer.resize(size).await.map_err(Into::into) } else { Ok(()) }
                            } else { Ok(()) } },
                        Err(error) => Err(error),
                    },
                    Action::Colors(value) => { dark = value;
                        if state == "active" { writer.update_colors(base_colors(value)).await.map_err(Into::into) } else { Ok(()) } },
                    Action::Select { source, row, column } if state == "active" => {
                        if !source.valid_for(&origin, input_epoch, &surface) { Err(failure("selection_changed")) }
                        else { navigation.select_source(Arc::clone(&source.page), source.offset, row, column) }
                    },
                    Action::Extend { source, row, column, anchor } if state == "active" => {
                        if !source.valid_for(&origin, input_epoch, &surface) { Err(failure("selection_changed")) }
                        else { navigation.extend_source(&source.page, source.offset, row, column, anchor) }
                    },
                    Action::Pointer { source, row, column, wheel_lines } if state == "active" && navigation.live() && !navigation.selected() => {
                        if !source.valid_for(&origin, input_epoch, &surface) || source.modes != surface.modes()
                            || source.pointer_mode == NativePointerMode::None || row >= surface.surface.size.rows || column >= surface.surface.size.columns {
                            Err(failure("input_not_ready"))
                        } else {
                            let bytes = encode_pointer(row, column, wheel_lines, surface.active_screen(), surface.modes());
                            if bytes.is_empty() { Ok(()) } else { writer.write_input(bytes).await.map_err(Into::into) }
                        }
                    },
                    Action::Scroll { offset, older, horizon } if state == "active" => {
                        match navigation.scroll(&surface, offset, older, horizon) {
                            Ok(request) => { query = request; prefetch = offset != 0; Ok(()) }, Err(error) => Err(error),
                        }
                    },
                    action @ (Action::Text { .. } | Action::Key { .. }) if (state == "active" || navigation.returning()) && command.input_epoch == input_epoch => {
                        let bytes = encode_action(action, surface.modes());
                        if bytes.len() > zterm_client::input::RESUME_INPUT_BOUND { Err(failure("resource_limit")) }
                        else if bytes.is_empty() { Ok(()) }
                        else if !navigation.live() {
                            match navigation.begin_return(&bytes) {
                                Ok(true) => { state = "synchronizing"; writer.request_sync(surface.revision()).await.map_err(Into::into) },
                                Ok(false) => Ok(()), Err(error) => Err(error),
                            }
                        } else { writer.write_input(bytes).await.map_err(Into::into) }
                    },
                    _ => Err(failure("input_not_ready")),
                };
                // Local gesture/resource errors do not terminate a healthy host Session.
                let _ = command.reply.send(result);
                if detach { break; }
                Ok(())
            }
            event = reader.read_event() => match event {
                Ok(Some(event)) => match event {
                    TerminalViewEvent::TransportState(value) => {
                        state = match value { TerminalViewTransportState::Active => "active",
                            TerminalViewTransportState::Reconnecting => "reconnecting", _ => "synchronizing" };
                        if state == "reconnecting" { navigation.disconnected(); input_epoch += 1; }
                        if state == "active" {
                            if let Some(bytes) = navigation.active() && !bytes.is_empty()
                                && writer.write_input(bytes).await.is_err() { navigation.reset(); input_epoch += 1; state = "reconnecting"; }
                            if desired_size != surface.surface.size { state = "synchronizing"; let _ = writer.resize(desired_size).await; }
                            let _ = writer.update_colors(base_colors(dark)).await;
                            prefetch = true;
                        }
                        Ok(())
                    },
                    TerminalViewEvent::Snapshot(snapshot) => match AttachmentSurface::from_snapshot(&snapshot) {
                        Ok(next) => { surface = next; navigation.observe(&surface);
                            navigation.snapshot_installed(); writer.snapshot_applied(surface.revision()).await.map_err(Into::into) },
                        Err(_) => Err(failure("malformed_frame")),
                    },
                    event @ (TerminalViewEvent::Delta(_) | TerminalViewEvent::ResumeDelta(_)) => {
                        let (delta, barrier) = match event {
                            TerminalViewEvent::Delta(delta) => (delta, false),
                            TerminalViewEvent::ResumeDelta(delta) => (delta, true),
                            _ => unreachable!("matched delta event"),
                        };
                        match surface.candidate_after_delta(&delta) {
                            Ok(Some(next)) => { surface = next; navigation.observe(&surface); writer.revision_applied(surface.revision()); prefetch = state == "active";
                                if barrier { navigation.snapshot_installed(); writer.snapshot_applied(surface.revision()).await.map_err(Into::into) } else { Ok(()) } },
                            Ok(None) => { state = "synchronizing"; writer.request_sync(surface.revision()).await.map_err(Into::into) },
                            Err(_) => Err(failure("malformed_frame")),
                        }
                    },
                    TerminalViewEvent::HistoryWindow(result) => match navigation.install(result) {
                        Ok(request) => { query = request; prefetch = true; Ok(()) }, Err(error) => Err(error),
                    },
                    TerminalViewEvent::SyncRequired { .. } => { state = "synchronizing"; Ok(()) },
                    TerminalViewEvent::LeaseLost { .. } => { navigation.disconnected(); input_epoch += 1; state = "lease_lost"; Ok(()) },
                    TerminalViewEvent::SessionEnded(_) => { navigation.disconnected(); input_epoch += 1; state = "ended"; Ok(()) },
                    TerminalViewEvent::ConnectionStatus(_) | TerminalViewEvent::ClipboardWrite(_) => Ok(()),
                },
                Ok(None) => break,
                Err(error) => Err(error.into()),
            }
        };
        if query.is_none() && prefetch && state == "active" {
            query = navigation.prefetch();
        }
        if let Some(query) = query {
            navigation.request_started();
            if writer.request_history_window(query).await.is_err() {
                navigation.request_failed();
                // Driver owns disconnection classification; never replay this query.
            }
        }
        generation += 1;
        let error = result.err().map(|error: NativeError| match error {
            NativeError::RequestFailed { code } => code,
            _ => "transport_unavailable".to_owned(),
        });
        if error.is_some() {
            state = "closed";
            navigation.disconnected();
            input_epoch += 1;
        }
        frame.send_replace(project_navigation(
            &surface,
            generation,
            state,
            error.clone(),
            dark,
            &mut navigation,
            input_epoch,
            &origin,
        ));
        if error.is_some() || matches!(state, "ended" | "lease_lost") {
            break;
        }
    }
    if !matches!(state, "ended" | "lease_lost") {
        let error = frame.borrow().error.clone();
        frame.send_replace(project_navigation(
            &surface,
            generation + 1,
            "closed",
            error,
            dark,
            &mut navigation,
            input_epoch + 1,
            &origin,
        ));
    }
    cancel.cancel();
}
fn encode_pointer(
    row: u16,
    column: u16,
    wheel_lines: i16,
    screen: ActiveScreen,
    modes: TerminalModes,
) -> Vec<u8> {
    use zterm_client::mouse::MouseInput;
    let report = MouseInput {
        code: 0,
        column: column + 1,
        row: row + 1,
        release: false,
    };
    if wheel_lines == 0 {
        [
            report,
            MouseInput {
                release: true,
                ..report
            },
        ]
        .into_iter()
        .filter_map(|event| event.route(screen, modes))
        .flatten()
        .collect()
    } else {
        let report = MouseInput {
            code: if wheel_lines > 0 { 64 } else { 65 },
            ..report
        };
        report
            .route(screen, modes)
            .unwrap_or_default()
            .repeat(usize::from(wheel_lines.unsigned_abs()))
    }
}
fn encode_action(action: Action, modes: TerminalModes) -> Vec<u8> {
    match action {
        Action::Text {
            text,
            modifiers,
            paste,
        } => {
            if paste && modes.bracketed_paste {
                let mut bytes = b"\x1b[200~".to_vec();
                bytes.extend_from_slice(text.as_bytes());
                bytes.extend_from_slice(b"\x1b[201~");
                return bytes;
            }
            if modifiers == 0
                && !modes
                    .keyboard_flags
                    .contains(TerminalKeyboardFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES)
            {
                return text.into_bytes();
            }
            if modifiers == 0 {
                let chars: Vec<_> = text.chars().collect();
                return KeyInput {
                    code: KeyCode::Unicode(0),
                    shifted: None,
                    modifiers,
                    kind: KeyEventKind::Press,
                    text: &chars,
                    raw: &[],
                }
                .encode(modes);
            }
            text.chars()
                .enumerate()
                .flat_map(|(index, character)| {
                    KeyInput {
                        code: KeyCode::Unicode(character as u32),
                        shifted: None,
                        modifiers: if index == 0 { modifiers } else { 0 },
                        kind: KeyEventKind::Press,
                        text: &[character],
                        raw: &[],
                    }
                    .encode(modes)
                })
                .collect()
        }
        Action::Key {
            key,
            modifiers,
            kind,
            text,
        } => {
            let chars: Vec<_> = text.chars().collect();
            KeyInput {
                code: key.code(),
                shifted: chars.first().copied(),
                modifiers,
                kind: match kind {
                    2 => KeyEventKind::Repeat,
                    3 => KeyEventKind::Release,
                    _ => KeyEventKind::Press,
                },
                text: &chars,
                raw: &[],
            }
            .encode(modes)
        }
        _ => Vec::new(),
    }
}
pub(crate) fn parse_session(value: &str) -> Result<SessionId, NativeError> {
    value.parse().map_err(|_| failure("session_not_found"))
}
pub(crate) fn failure(code: &str) -> NativeError {
    NativeError::RequestFailed {
        code: code.to_owned(),
    }
}

pub(crate) fn base_colors(dark: bool) -> TerminalColorProfile {
    let mut profile = TerminalColorProfile {
        appearance: if dark {
            TerminalAppearance::Dark
        } else {
            TerminalAppearance::Light
        },
        ..Default::default()
    };
    for slot in 0..TERMINAL_COLOR_SLOTS {
        let color = fallback(slot, dark);
        profile.values[slot] =
            TerminalColorValue::Rgb((color >> 16) as u8, (color >> 8) as u8, color as u8);
    }
    profile
}
fn fallback(slot: usize, dark: bool) -> u32 {
    let rgb = match slot {
        0..=15 => [
            0x1a241d, 0xe88981, 0xa9dcad, 0xf2ce91, 0xaecffa, 0xdab4ef, 0x99d8da, 0xdce4dc,
            0x7a8f80, 0xffb4ab, 0xd9fba7, 0xffe1aa, 0xc6ddff, 0xedcaff, 0xbaf5f5, 0xf0f4ed,
        ][slot],
        16..=231 => {
            let n = slot - 16;
            let level = |v: usize| if v == 0 { 0 } else { 55 + v * 40 };
            ((level(n / 36) << 16) | (level(n / 6 % 6) << 8) | level(n % 6)) as u32
        }
        232..=255 => {
            let v = (8 + (slot - 232) * 10) as u32;
            v * 0x010101
        }
        COLOR_BACKGROUND | COLOR_CURSOR_TEXT => {
            if dark {
                0x101713
            } else {
                0xf5f8f3
            }
        }
        COLOR_SELECTION_BACKGROUND => {
            if dark {
                0x365640
            } else {
                0xd4e6c2
            }
        }
        _ => {
            if dark {
                0xf0f4ed
            } else {
                0x182219
            }
        }
    };
    0xff000000 | rgb
}
fn slot_color(colors: &TerminalColorSnapshot, slot: usize, dark: bool) -> u32 {
    match colors.profile.values[slot] {
        TerminalColorValue::Rgb(r, g, b) => {
            0xff000000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
        }
        _ => fallback(colors.source(slot), dark),
    }
}
fn color(value: TerminalColor, default: usize, colors: &TerminalColorSnapshot, dark: bool) -> u32 {
    match value {
        TerminalColor::Default => slot_color(colors, default, dark),
        TerminalColor::Indexed(index) => slot_color(colors, usize::from(index), dark),
        TerminalColor::Rgb(r, g, b) => {
            0xff000000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
        }
    }
}
fn project(
    surface: &AttachmentSurface,
    generation: u64,
    state: &str,
    error: Option<String>,
    dark: bool,
    rows: &[TerminalSurfaceRow],
) -> NativeFrame {
    let surface = &surface.surface;
    NativeFrame {
        pointer_mode: NativePointerMode::None,
        stats: NativeNavigationStats::default(),
        notice: None,
        source: None,
        input_epoch: 1,
        input_ready: state == "active",
        history_offset: 0,
        first_row: surface
            .scroll_metrics
            .map_or(0, |metrics| metrics.max_offset_from_bottom as i64),
        selection: None,
        generation,
        state: state.to_owned(),
        error,
        viewport: NativeViewport {
            rows: surface.size.rows,
            columns: surface.size.columns,
        },
        rows: project_rows(rows, &surface.colors, dark),
        cursor_row: surface.cursor.row,
        cursor_column: surface.cursor.column,
        cursor_visible: surface.cursor.visible && state == "active",
        cursor_color: slot_color(&surface.colors, COLOR_CURSOR, dark),
        background: slot_color(&surface.colors, COLOR_BACKGROUND, dark),
        history_maximum: surface
            .scroll_metrics
            .map_or(0, |metrics| metrics.max_offset_from_bottom),
    }
}

fn project_rows(
    rows: &[TerminalSurfaceRow],
    colors: &TerminalColorSnapshot,
    dark: bool,
) -> Vec<NativeRow> {
    rows.iter()
        .map(|row| NativeRow {
            wrapped: row.wrapped,
            cells: row
                .cells
                .iter()
                .map(|cell| {
                    let mut foreground =
                        color(cell.style.foreground, COLOR_FOREGROUND, colors, dark);
                    let mut background =
                        color(cell.style.background, COLOR_BACKGROUND, colors, dark);
                    if cell.style.inverse {
                        std::mem::swap(&mut foreground, &mut background);
                    }
                    NativeCell {
                        text: cell.contents.clone(),
                        width: if cell.wide_continuation {
                            0
                        } else if cell.wide {
                            2
                        } else {
                            1
                        },
                        foreground,
                        background,
                        attributes: u8::from(cell.style.bold)
                            | (u8::from(cell.style.dim) << 1)
                            | (u8::from(cell.style.italic) << 2),
                        underline: match cell.style.underline {
                            TerminalUnderline::None => 0,
                            TerminalUnderline::Single => 1,
                            TerminalUnderline::Double => 2,
                            TerminalUnderline::Curly => 3,
                            TerminalUnderline::Dotted => 4,
                            TerminalUnderline::Dashed => 5,
                        },
                        underline_color: if cell.style.underline_color == TerminalColor::Default {
                            foreground
                        } else {
                            color(cell.style.underline_color, COLOR_FOREGROUND, colors, dark)
                        },
                    }
                })
                .collect(),
        })
        .collect()
}
#[allow(clippy::too_many_arguments)]
fn project_navigation(
    surface: &AttachmentSurface,
    generation: u64,
    state: &str,
    error: Option<String>,
    dark: bool,
    navigation: &mut Navigation,
    input_epoch: u64,
    origin: &Arc<()>,
) -> NativeFrame {
    let frozen_rows = (!navigation.live()).then(|| navigation.rows()).flatten();
    let mut frame = project(
        surface,
        generation,
        state,
        error,
        dark,
        frozen_rows.unwrap_or(&surface.surface.rows),
    );
    frame.input_epoch = input_epoch;
    frame.input_ready = state == "active" || (state == "synchronizing" && navigation.returning());
    frame.history_offset = navigation.offset();
    if state == "active" && navigation.live() && !navigation.selected() {
        frame.pointer_mode = NativePointerMode::for_surface(surface);
    }
    if frozen_rows.is_some() {
        frame.cursor_visible = false;
        frame.first_row = navigation.first_row().unwrap_or(frame.first_row);
    }
    if frame.input_ready
        && let Some((page, offset)) = navigation.frame_source(surface)
    {
        frame.source = Some(Arc::new(NativeFrameSource {
            page,
            offset,
            input_epoch,
            screen: surface.active_screen(),
            origin: Arc::clone(origin),
            modes: surface.modes(),
            pointer_mode: frame.pointer_mode,
        }));
    }
    frame.selection = navigation
        .endpoints()
        .map(|(anchor, focus)| NativeSelection {
            anchor_row: anchor.row,
            anchor_column: anchor.column,
            focus_row: focus.row,
            focus_column: focus.column,
        });
    frame.stats = navigation.stats();
    frame.notice = navigation.notice.map(str::to_owned);
    frame
}

impl NativeFrameSource {
    fn valid_for(&self, origin: &Arc<()>, input_epoch: u64, surface: &AttachmentSurface) -> bool {
        Arc::ptr_eq(&self.origin, origin)
            && self.input_epoch == input_epoch
            && self.screen == surface.active_screen()
            && self.page.anchor.viewport == surface.surface.size
    }
}
