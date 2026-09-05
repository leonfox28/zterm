//! Typed frontend commands and events above the single Session client owner.
#[cfg(unix)]
use super::{
    LocalAttachmentEvent, SessionClient, is_attachment_command_stream_closed,
    is_attachment_stream_closed_without_event,
};
use crate::error::DaemonError;
use std::fmt;
#[cfg(unix)]
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use zterm_core::terminal::{
    TerminalClipboardWrite, TerminalHistoryWindowQuery, TerminalSize, TerminalSurfaceDelta,
    TerminalSurfaceHistoryWindowResult, TerminalSurfaceSnapshot,
};
use zterm_core::{AttachmentId, DomainErrorKind, Revision, SessionId};
#[cfg(not(unix))]
fn unsupported_command_platform() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::UnsupportedPlatform,
        "terminal views require Unix",
    )
}
#[cfg(unix)]
const TERMINAL_DRIVER_CAPACITY: usize = 8;
#[cfg(unix)]
const TERMINAL_CLOSURE_CORRELATION_WINDOW: Duration = Duration::from_millis(100);

/// Replacement state consumed by the one CLI compositor.
pub type TerminalViewSnapshot = TerminalSurfaceSnapshot;

/// Contiguous semantic update from one exact acknowledged revision.
pub type TerminalViewDelta = TerminalSurfaceDelta;

/// Monotonic attachment transport state owned by the frontend Session client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalViewTransportState {
    /// The frontend is acquiring the attachment transport.
    Preparing,
    /// A full state or resume delta is awaiting exact acknowledgement.
    Synchronizing,
    /// Input and resize may be sent.
    Active,
    /// The frontend is replacing a lost remote stream.
    Reconnecting,
}

/// Address-free selected network path for one remote terminal view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalViewConnectionPath {
    /// No current selected path has been observed.
    Unknown,
    /// Iroh selected a direct IP path.
    Direct,
    /// Iroh selected a relay path.
    Relay,
}

/// Immutable route chosen before opening one terminal view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalViewRoute {
    /// One direct same-UID Session IPC stream to this daemon.
    Local,
    /// One same-UID opaque tunnel to a remote daemon Session stream.
    Remote,
}

/// Redaction-safe presentation metadata frozen with the exact resolved target.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalViewTarget {
    pub(crate) display_name: String,
    pub(crate) route: TerminalViewRoute,
}

impl fmt::Debug for TerminalViewTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewTarget")
            .field("display_name", &"[REDACTED]")
            .field("display_name_len", &self.display_name.len())
            .field("route", &self.route)
            .finish()
    }
}

impl TerminalViewTarget {
    /// Builds presentation-only metadata. This value contains no routing
    /// authority; Session target selection remains frozen separately.
    #[must_use]
    pub fn for_display(display_name: impl Into<String>, route: TerminalViewRoute) -> Self {
        Self {
            display_name: display_name.into(),
            route,
        }
    }

    /// Frozen configured device name or exact remote alias for presentation.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Explicit transport route; display text never selects behavior.
    #[must_use]
    pub const fn route(&self) -> TerminalViewRoute {
        self.route
    }
}

/// Current selected path and RTT for a remote terminal view.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalViewConnectionStatus {
    path: TerminalViewConnectionPath,
    rtt_ms: Option<u32>,
}

impl fmt::Debug for TerminalViewConnectionStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewConnectionStatus")
            .field("path", &self.path)
            .field("rtt_ms", &self.rtt_ms)
            .finish()
    }
}

impl TerminalViewConnectionStatus {
    /// Current selected Iroh path class.
    #[must_use]
    pub const fn path(&self) -> TerminalViewConnectionPath {
        self.path
    }

    /// Current selected-path round-trip time in integer milliseconds.
    #[must_use]
    pub const fn rtt_ms(&self) -> Option<u32> {
        self.rtt_ms
    }
}

/// Stable reason that the daemon-owned Session ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalViewEndReason {
    /// The root shell exited naturally.
    NaturalExit,
    /// An explicit Session close ended the PTY.
    ExplicitClose,
    /// Daemon shutdown ended the PTY.
    DaemonStop,
    /// The retained terminal driver failed.
    DriverFailure,
}

/// Terminal lifecycle details containing no rendered terminal bytes.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalViewEnded {
    /// Stable Session end reason.
    pub reason: TerminalViewEndReason,
    /// Exit code when the platform reported one.
    pub exit_code: u32,
    /// Bounded platform signal name when reported.
    pub signal: String,
}

/// Stateless renderer-neutral history-window response.
pub type TerminalViewHistoryWindow = TerminalSurfaceHistoryWindowResult;

impl fmt::Debug for TerminalViewEnded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewEnded")
            .field("reason", &self.reason)
            .field("exit_code", &self.exit_code)
            .field("has_signal", &!self.signal.is_empty())
            .finish_non_exhaustive()
    }
}

/// One typed event from the frontend-owned terminal driver.
#[derive(Clone, Eq, PartialEq)]
pub enum TerminalViewEvent {
    /// Attachment transport state changed.
    TransportState(TerminalViewTransportState),
    /// Selected path and RTT changed for this remote view.
    ConnectionStatus(TerminalViewConnectionStatus),
    /// Replace the local rendered state atomically.
    Snapshot(TerminalViewSnapshot),
    /// Apply one merged update only when its baseline is contiguous.
    Delta(TerminalViewDelta),
    /// Correlated reconnect barrier; acknowledge once after successful application.
    ResumeDelta(TerminalViewDelta),
    /// One correlated stateless bounded history-window outcome.
    HistoryWindow(TerminalViewHistoryWindow),
    /// One validated latest-only child clipboard write.
    ClipboardWrite(TerminalClipboardWrite),
    /// The following snapshot replaces the current live rendering baseline.
    SyncRequired {
        /// Latest host revision declared by the synchronization marker.
        latest_revision: Revision,
    },
    /// Another controller replaced this attachment.
    LeaseLost {
        /// Controller generation which replaced this view.
        generation: u64,
    },
    /// The daemon-owned Session and PTY ended.
    SessionEnded(TerminalViewEnded),
}

impl fmt::Debug for TerminalViewEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TransportState(state) => formatter
                .debug_tuple("TransportState")
                .field(state)
                .finish(),
            Self::ConnectionStatus(status) => formatter
                .debug_tuple("ConnectionStatus")
                .field(status)
                .finish(),
            Self::Snapshot(snapshot) => formatter.debug_tuple("Snapshot").field(snapshot).finish(),
            Self::Delta(delta) => formatter.debug_tuple("Delta").field(delta).finish(),
            Self::ResumeDelta(delta) => formatter.debug_tuple("ResumeDelta").field(delta).finish(),
            Self::HistoryWindow(window) => formatter
                .debug_tuple("HistoryWindow")
                .field(window)
                .finish(),
            Self::ClipboardWrite(write) => formatter
                .debug_tuple("ClipboardWrite")
                .field(write)
                .finish(),
            Self::SyncRequired { latest_revision } => formatter
                .debug_struct("SyncRequired")
                .field("latest_revision", latest_revision)
                .finish(),
            Self::LeaseLost { generation } => formatter
                .debug_struct("LeaseLost")
                .field("generation", generation)
                .finish(),
            Self::SessionEnded(ended) => {
                formatter.debug_tuple("SessionEnded").field(ended).finish()
            }
        }
    }
}

/// Opaque prepared terminal view handed to the raw-terminal UI.
///
/// The socket, frame codec, target token, operation lease, and takeover replay
/// owner remain private to the daemon crate.
pub struct PreparedTerminalView {
    session_id: SessionId,
    attachment_id: AttachmentId,
    initial_snapshot: TerminalViewSnapshot,
    takeover: bool,
    target: TerminalViewTarget,
    #[cfg(unix)]
    client: SessionClient,
}

impl fmt::Debug for PreparedTerminalView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedTerminalView")
            .field("session_id", &self.session_id)
            .field("attachment_id", &self.attachment_id)
            .field("initial_revision", &self.initial_snapshot.revision)
            .field("takeover", &self.takeover)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl PreparedTerminalView {
    #[cfg(unix)]
    pub(crate) fn new(
        mut client: SessionClient,
        takeover: bool,
        target: TerminalViewTarget,
    ) -> Result<Self, DaemonError> {
        let initial_snapshot = client.take_initial_snapshot().ok_or_else(|| {
            terminal_protocol_error("prepared attachment already transferred its initial snapshot")
        })?;
        Ok(Self {
            session_id: client.session_id(),
            attachment_id: client.attachment_id(),
            initial_snapshot,
            takeover,
            target,
            client,
        })
    }

    /// Stable Session selected by this prepared view.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Target-issued attachment identity for this initial stream epoch.
    #[must_use]
    pub const fn attachment_id(&self) -> AttachmentId {
        self.attachment_id
    }

    /// Revision of the initial authoritative snapshot retained internally.
    #[must_use]
    pub const fn initial_revision(&self) -> Revision {
        self.initial_snapshot.revision
    }

    /// Initial authoritative state which must be flushed before acknowledgement.
    #[must_use]
    pub const fn initial_snapshot(&self) -> &TerminalViewSnapshot {
        &self.initial_snapshot
    }

    /// Frozen route and redaction-safe device label used by universal chrome.
    #[must_use]
    pub const fn target(&self) -> &TerminalViewTarget {
        &self.target
    }

    /// Acknowledges the exact flushed initial state and starts one typed driver.
    ///
    /// A requested takeover is initiated before the driver starts, so the same
    /// owner retains its daemon-issued operation lease, response correlation,
    /// decoder, and socket for the complete transition.
    #[cfg(unix)]
    pub async fn acknowledge_initial(mut self) -> Result<TerminalViewIo, DaemonError> {
        self.client
            .snapshot_applied(self.initial_snapshot.revision)
            .await?;
        if self.takeover {
            self.client.begin_takeover().await?;
        }
        let initial_state = if self.client.reconnect_pending() {
            TerminalViewTransportState::Reconnecting
        } else if self.takeover {
            TerminalViewTransportState::Synchronizing
        } else {
            TerminalViewTransportState::Active
        };
        Ok(spawn_terminal_driver(
            self.client,
            initial_state,
            self.target.route,
            self.takeover,
        ))
    }

    /// Current non-Unix milestone has no raw terminal implementation.
    #[cfg(not(unix))]
    pub async fn acknowledge_initial(self) -> Result<TerminalViewIo, DaemonError> {
        let _ = self;
        Err(unsupported_command_platform())
    }
}

/// Running typed terminal view, split only above the single daemon driver owner.
pub struct TerminalViewIo {
    reader: TerminalViewEventReader,
    writer: TerminalViewCommandWriter,
}

impl fmt::Debug for TerminalViewIo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewIo")
            .finish_non_exhaustive()
    }
}

impl TerminalViewIo {
    /// Separates typed event consumption from typed command submission.
    #[must_use]
    pub fn split(self) -> (TerminalViewEventReader, TerminalViewCommandWriter) {
        (self.reader, self.writer)
    }
}

/// Bounded event side of one frontend-owned terminal driver.
pub struct TerminalViewEventReader {
    #[cfg(unix)]
    receiver: tokio::sync::mpsc::Receiver<Result<TerminalViewEvent, DaemonError>>,
    #[cfg(unix)]
    clipboard: Arc<TerminalClipboardSlot>,
    #[cfg(unix)]
    clipboard_wakeup: tokio::sync::watch::Receiver<()>,
}

impl fmt::Debug for TerminalViewEventReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewEventReader")
            .finish_non_exhaustive()
    }
}

impl TerminalViewEventReader {
    /// Waits for the next typed event; active reads have no synthetic timeout.
    pub async fn read_event(&mut self) -> Result<Option<TerminalViewEvent>, DaemonError> {
        #[cfg(unix)]
        {
            loop {
                tokio::select! {
                    biased;
                    event = self.receiver.recv() => {
                        if event.is_none() {
                            self.clipboard.clear();
                        }
                        return event.transpose();
                    }
                    changed = self.clipboard_wakeup.changed() => {
                        if changed.is_err() {
                            continue;
                        }
                        self.clipboard_wakeup.borrow_and_update();
                        if let Some(write) = self.clipboard.take() {
                            return Ok(Some(TerminalViewEvent::ClipboardWrite(write)));
                        }
                    }
                }
            }
        }
        #[cfg(not(unix))]
        {
            Err(unsupported_command_platform())
        }
    }
}

#[cfg(unix)]
struct TerminalClipboardSlot {
    pending: Mutex<Option<TerminalClipboardWrite>>,
    wake: tokio::sync::watch::Sender<()>,
}

#[cfg(unix)]
impl TerminalClipboardSlot {
    fn new() -> (Arc<Self>, tokio::sync::watch::Receiver<()>) {
        let (wake, receiver) = tokio::sync::watch::channel(());
        (
            Arc::new(Self {
                pending: Mutex::new(None),
                wake,
            }),
            receiver,
        )
    }

    fn replace(&self, write: TerminalClipboardWrite) {
        *self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(write);
        self.wake.send_replace(());
    }

    fn take(&self) -> Option<TerminalClipboardWrite> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    fn clear(&self) {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}

/// Cloneable typed command side of one frontend-owned terminal driver.
#[derive(Clone)]
pub struct TerminalViewCommandWriter {
    #[cfg(unix)]
    sender: tokio::sync::mpsc::Sender<PendingTerminalCommand>,
    #[cfg(unix)]
    terminal_outcome_queued: tokio::sync::watch::Receiver<bool>,
    #[cfg(unix)]
    applied_revision: Arc<AtomicU64>,
}

impl fmt::Debug for TerminalViewCommandWriter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewCommandWriter")
            .finish_non_exhaustive()
    }
}

impl TerminalViewCommandWriter {
    /// Records the exact revision successfully installed by the frontend.
    /// This is local state for remote resume and sends no Session frame.
    pub fn revision_applied(&self, revision: Revision) {
        #[cfg(unix)]
        self.applied_revision
            .fetch_max(revision.get(), Ordering::AcqRel);
        #[cfg(not(unix))]
        let _ = revision;
    }

    /// Acknowledges an exactly flushed replacement snapshot or resume delta.
    pub async fn snapshot_applied(&self, revision: Revision) -> Result<(), DaemonError> {
        #[cfg(unix)]
        {
            self.submit(TerminalDriverCommand::SnapshotApplied { revision })
                .await
        }
        #[cfg(not(unix))]
        {
            let _ = revision;
            Err(unsupported_command_platform())
        }
    }

    /// Sends ordinary controller input. Callers must gate this on `Active`.
    pub async fn write_input(&self, bytes: Vec<u8>) -> Result<(), DaemonError> {
        #[cfg(unix)]
        {
            self.submit(TerminalDriverCommand::Input { bytes }).await
        }
        #[cfg(not(unix))]
        {
            let _ = bytes;
            Err(unsupported_command_platform())
        }
    }

    /// Sends the latest validated viewport. Callers coalesce while non-active.
    pub async fn resize(&self, size: TerminalSize) -> Result<(), DaemonError> {
        #[cfg(unix)]
        {
            self.submit(TerminalDriverCommand::Resize { size }).await
        }
        #[cfg(not(unix))]
        {
            let _ = size;
            Err(unsupported_command_platform())
        }
    }

    /// Requests a full replacement after a revision gap.
    pub async fn request_sync(&self, known_revision: Revision) -> Result<(), DaemonError> {
        #[cfg(unix)]
        {
            self.submit(TerminalDriverCommand::RequestSync { known_revision })
                .await
        }
        #[cfg(not(unix))]
        {
            let _ = known_revision;
            Err(unsupported_command_platform())
        }
    }

    /// Requests one stateless bounded history window.
    pub async fn request_history_window(
        &self,
        query: TerminalHistoryWindowQuery,
    ) -> Result<(), DaemonError> {
        #[cfg(unix)]
        {
            self.submit(TerminalDriverCommand::RequestHistoryWindow { query })
                .await
        }
        #[cfg(not(unix))]
        {
            let _ = query;
            Err(unsupported_command_platform())
        }
    }

    /// Detaches this view while leaving the Session and PTY running.
    pub async fn detach(&self) -> Result<(), DaemonError> {
        #[cfg(unix)]
        {
            self.submit(TerminalDriverCommand::Detach).await
        }
        #[cfg(not(unix))]
        {
            Err(unsupported_command_platform())
        }
    }

    #[cfg(unix)]
    async fn submit(&self, command: TerminalDriverCommand) -> Result<(), DaemonError> {
        let deadline = super::control_deadline();
        let (response, receiver) = tokio::sync::oneshot::channel();
        let pending = PendingTerminalCommand {
            command,
            deadline,
            response,
        };
        tokio::time::timeout_at(deadline, async {
            if self.sender.send(pending).await.is_err() {
                return self.correlate_terminal_outcome().await;
            }
            match receiver.await {
                Ok(result) => result,
                Err(_) => self.correlate_terminal_outcome().await,
            }
        })
        .await
        .unwrap_or_else(|_| Err(super::control_timeout()))
    }

    #[cfg(unix)]
    async fn correlate_terminal_outcome(&self) -> Result<(), DaemonError> {
        let mut queued = self.terminal_outcome_queued.clone();
        if *queued.borrow_and_update() {
            return Ok(());
        }
        match tokio::time::timeout(TERMINAL_CLOSURE_CORRELATION_WINDOW, queued.changed()).await {
            Ok(Ok(())) if *queued.borrow_and_update() => Ok(()),
            Ok(Ok(()) | Err(_)) | Err(_) => Err(terminal_command_outcome_unavailable()),
        }
    }
}

#[cfg(unix)]
struct PendingTerminalCommand {
    command: TerminalDriverCommand,
    deadline: tokio::time::Instant,
    response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
}

#[cfg(unix)]
enum TerminalDriverCommand {
    SnapshotApplied { revision: Revision },
    Input { bytes: Vec<u8> },
    Resize { size: TerminalSize },
    RequestSync { known_revision: Revision },
    RequestHistoryWindow { query: TerminalHistoryWindowQuery },
    Detach,
}

#[cfg(unix)]
struct TerminalDriverInitial {
    state: TerminalViewTransportState,
    route: TerminalViewRoute,
    takeover: bool,
}

#[cfg(unix)]
fn spawn_terminal_driver(
    client: SessionClient,
    initial_state: TerminalViewTransportState,
    route: TerminalViewRoute,
    takeover: bool,
) -> TerminalViewIo {
    let applied_revision = client.applied_revision_tracker();
    let (command_sender, command_receiver) = tokio::sync::mpsc::channel(TERMINAL_DRIVER_CAPACITY);
    let (event_sender, event_receiver) = tokio::sync::mpsc::channel(TERMINAL_DRIVER_CAPACITY);
    let (clipboard, clipboard_wakeup) = TerminalClipboardSlot::new();
    let (terminal_outcome_sender, terminal_outcome_receiver) = tokio::sync::watch::channel(false);
    tokio::spawn(run_terminal_driver(
        client,
        command_receiver,
        event_sender,
        Arc::clone(&clipboard),
        terminal_outcome_sender,
        TerminalDriverInitial {
            state: initial_state,
            route,
            takeover,
        },
    ));
    TerminalViewIo {
        reader: TerminalViewEventReader {
            receiver: event_receiver,
            clipboard,
            clipboard_wakeup,
        },
        writer: TerminalViewCommandWriter {
            sender: command_sender,
            terminal_outcome_queued: terminal_outcome_receiver,
            applied_revision,
        },
    }
}

#[cfg(unix)]
async fn run_terminal_driver(
    mut client: SessionClient,
    mut commands: tokio::sync::mpsc::Receiver<PendingTerminalCommand>,
    events: tokio::sync::mpsc::Sender<Result<TerminalViewEvent, DaemonError>>,
    clipboard: Arc<TerminalClipboardSlot>,
    terminal_outcome_queued: tokio::sync::watch::Sender<bool>,
    initial: TerminalDriverInitial,
) {
    use std::collections::VecDeque;

    let TerminalDriverInitial {
        state: initial_state,
        route,
        takeover,
    } = initial;
    let mut pending = VecDeque::from([Ok(TerminalViewEvent::TransportState(initial_state))]);
    if route == TerminalViewRoute::Remote {
        pending.push_back(Ok(TerminalViewEvent::ConnectionStatus(
            TerminalViewConnectionStatus {
                path: TerminalViewConnectionPath::Unknown,
                rtt_ms: None,
            },
        )));
    }
    let mut stop_after_pending = false;
    let mut local_takeover_pending = takeover;
    let mut last_state = initial_state;

    loop {
        if pending.is_empty() {
            tokio::select! {
                command = commands.recv() => {
                    if apply_terminal_driver_command(
                        command,
                        &mut client,
                        &mut pending,
                        route,
                        &mut local_takeover_pending,
                        &mut last_state,
                        &mut stop_after_pending,
                        &clipboard,
                    ).await {
                        clipboard.clear();
                        return;
                    }
                }
                () = events.closed() => {
                    clipboard.clear();
                    let _ = client.detach().await;
                    return;
                }
                event = client.read_next_event() => {
                    if queue_local_attachment_event(
                        event,
                        &mut pending,
                        route,
                        &mut local_takeover_pending,
                        &mut last_state,
                        &clipboard,
                    ) {
                        stop_after_pending = true;
                    }
                }
            }
            continue;
        }

        tokio::select! {
            command = commands.recv() => {
                if apply_terminal_driver_command(
                    command,
                    &mut client,
                    &mut pending,
                    route,
                    &mut local_takeover_pending,
                    &mut last_state,
                    &mut stop_after_pending,
                    &clipboard,
                ).await {
                    clipboard.clear();
                    return;
                }
            }
            permit = events.reserve() => {
                let Ok(permit) = permit else {
                    clipboard.clear();
                    let _ = client.detach().await;
                    return;
                };
                let event = pending.pop_front().expect("pending event was checked above");
                permit.send(event);
                if pending.is_empty() && stop_after_pending {
                    clipboard.clear();
                    terminal_outcome_queued.send_replace(true);
                    return;
                }
            }
        }
    }
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
async fn apply_terminal_driver_command(
    command: Option<PendingTerminalCommand>,
    client: &mut SessionClient,
    pending: &mut std::collections::VecDeque<Result<TerminalViewEvent, DaemonError>>,
    route: TerminalViewRoute,
    local_takeover_pending: &mut bool,
    last_state: &mut TerminalViewTransportState,
    stop_after_pending: &mut bool,
    clipboard: &TerminalClipboardSlot,
) -> bool {
    match handle_terminal_driver_command(command, client).await {
        TerminalDriverCommandResult::Continue => false,
        TerminalDriverCommandResult::SnapshotApplied
            if !client.reconnect_pending()
                && !*local_takeover_pending
                && *last_state != TerminalViewTransportState::Active =>
        {
            *last_state = TerminalViewTransportState::Active;
            pending.push_back(Ok(TerminalViewEvent::TransportState(*last_state)));
            false
        }
        TerminalDriverCommandResult::SnapshotApplied => false,
        TerminalDriverCommandResult::CommandStreamClosed { response } => {
            correlate_terminal_command_closure(
                client,
                pending,
                route,
                local_takeover_pending,
                last_state,
                response,
                clipboard,
            )
            .await;
            *stop_after_pending = true;
            false
        }
        TerminalDriverCommandResult::Stop => true,
    }
}

#[cfg(unix)]
async fn correlate_terminal_command_closure(
    client: &mut SessionClient,
    pending: &mut std::collections::VecDeque<Result<TerminalViewEvent, DaemonError>>,
    route: TerminalViewRoute,
    local_takeover_pending: &mut bool,
    last_state: &mut TerminalViewTransportState,
    response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
    clipboard: &TerminalClipboardSlot,
) {
    let deadline = tokio::time::Instant::now() + TERMINAL_CLOSURE_CORRELATION_WINDOW;
    loop {
        let event = match tokio::time::timeout_at(deadline, client.read_next_event()).await {
            Ok(Ok(event)) => Ok(event),
            Ok(Err(error)) if !is_attachment_stream_closed_without_event(&error) => Err(error),
            Ok(Err(_)) | Err(_) => {
                pending.push_back(Err(terminal_command_outcome_unavailable()));
                break;
            }
        };
        if queue_local_attachment_event(
            event,
            pending,
            route,
            local_takeover_pending,
            last_state,
            clipboard,
        ) {
            break;
        }
    }
    // The event side now owns the only user-visible terminal outcome, avoiding
    // a race between a raw write failure and a queued typed lifecycle event.
    let _ = response.send(Ok(()));
}

#[cfg(unix)]
fn terminal_command_outcome_unavailable() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DaemonStopped,
        "terminal attachment closed while a command was in flight",
    )
}

#[cfg(unix)]
fn queue_local_attachment_event(
    event: Result<LocalAttachmentEvent, DaemonError>,
    pending: &mut std::collections::VecDeque<Result<TerminalViewEvent, DaemonError>>,
    route: TerminalViewRoute,
    local_takeover_pending: &mut bool,
    last_state: &mut TerminalViewTransportState,
    clipboard: &TerminalClipboardSlot,
) -> bool {
    match event {
        Ok(LocalAttachmentEvent::ClipboardWrite(write)) => {
            clipboard.replace(write);
            false
        }
        Ok(LocalAttachmentEvent::Takeover(_)) if *local_takeover_pending => {
            *local_takeover_pending = false;
            *last_state = TerminalViewTransportState::Active;
            pending.push_back(Ok(TerminalViewEvent::TransportState(*last_state)));
            false
        }
        Ok(LocalAttachmentEvent::Takeover(_)) => false,
        Ok(LocalAttachmentEvent::TransportState(state)) => {
            match terminal_transport_state_from_wire(state.state) {
                Ok(TerminalViewTransportState::Preparing) => false,
                Ok(TerminalViewTransportState::Reconnecting) => {
                    clipboard.clear();
                    if *last_state == TerminalViewTransportState::Reconnecting {
                        false
                    } else {
                        *last_state = TerminalViewTransportState::Reconnecting;
                        pending.push_back(Ok(TerminalViewEvent::TransportState(*last_state)));
                        false
                    }
                }
                Ok(state) if state == *last_state => false,
                Ok(state) => {
                    *last_state = state;
                    pending.push_back(Ok(TerminalViewEvent::TransportState(state)));
                    false
                }
                Err(error) => {
                    clipboard.clear();
                    pending.push_back(Err(error));
                    true
                }
            }
        }
        Ok(event) => {
            let terminal = matches!(
                event,
                LocalAttachmentEvent::LeaseLost(_) | LocalAttachmentEvent::SessionEnded(_)
            );
            if terminal {
                clipboard.clear();
            }
            if local_event_requires_synchronizing(&event)
                && *last_state != TerminalViewTransportState::Synchronizing
            {
                *last_state = TerminalViewTransportState::Synchronizing;
                pending.push_back(Ok(TerminalViewEvent::TransportState(*last_state)));
            }
            match terminal_event_from_local(event, route) {
                Ok(Some(event)) => pending.push_back(Ok(event)),
                Ok(None) => {}
                Err(error) => {
                    pending.push_back(Err(error));
                    return true;
                }
            }
            terminal
        }
        Err(error) => {
            clipboard.clear();
            pending.push_back(Err(error));
            true
        }
    }
}

#[cfg(unix)]
async fn handle_terminal_driver_command(
    command: Option<PendingTerminalCommand>,
    client: &mut SessionClient,
) -> TerminalDriverCommandResult {
    let Some(PendingTerminalCommand {
        command,
        deadline,
        response,
    }) = command
    else {
        let _ = client.detach().await;
        return TerminalDriverCommandResult::Stop;
    };
    if deadline <= tokio::time::Instant::now() || response.is_closed() {
        let _ = response.send(Err(super::control_timeout()));
        return TerminalDriverCommandResult::Continue;
    }
    let success = match command {
        TerminalDriverCommand::SnapshotApplied { .. } => {
            TerminalDriverCommandResult::SnapshotApplied
        }
        TerminalDriverCommand::Detach => TerminalDriverCommandResult::Stop,
        _ => TerminalDriverCommandResult::Continue,
    };
    let result = tokio::time::timeout_at(deadline, async {
        match command {
            TerminalDriverCommand::SnapshotApplied { revision } => {
                client.snapshot_applied(revision).await
            }
            TerminalDriverCommand::Input { bytes } => client.write_input(bytes).await,
            TerminalDriverCommand::Resize { size } => client.resize(size).await,
            TerminalDriverCommand::RequestSync { known_revision } => {
                client.request_sync(known_revision).await
            }
            TerminalDriverCommand::RequestHistoryWindow { query } => {
                client.request_history_window(query).await
            }
            TerminalDriverCommand::Detach => client.detach().await,
        }
    })
    .await
    .unwrap_or_else(|_| {
        client.invalidate_transport();
        Err(super::control_timeout())
    });
    if result
        .as_ref()
        .err()
        .is_some_and(is_attachment_command_stream_closed)
    {
        if matches!(&success, TerminalDriverCommandResult::Stop) {
            let _ = response.send(Ok(()));
            return TerminalDriverCommandResult::Stop;
        }
        return TerminalDriverCommandResult::CommandStreamClosed { response };
    }
    let failed = result.is_err();
    let _ = response.send(result);
    if failed {
        TerminalDriverCommandResult::Stop
    } else {
        success
    }
}

#[cfg(unix)]
enum TerminalDriverCommandResult {
    Continue,
    SnapshotApplied,
    CommandStreamClosed {
        response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
    },
    Stop,
}

#[cfg(unix)]
fn local_event_requires_synchronizing(event: &LocalAttachmentEvent) -> bool {
    matches!(
        event,
        LocalAttachmentEvent::Snapshot(_)
            | LocalAttachmentEvent::ResumeDelta(_)
            | LocalAttachmentEvent::SyncRequired(_)
    )
}

#[cfg(unix)]
fn terminal_event_from_local(
    event: LocalAttachmentEvent,
    route: TerminalViewRoute,
) -> Result<Option<TerminalViewEvent>, DaemonError> {
    match event {
        LocalAttachmentEvent::Snapshot(snapshot) => Ok(Some(TerminalViewEvent::Snapshot(snapshot))),
        LocalAttachmentEvent::Delta(delta) => Ok(Some(TerminalViewEvent::Delta(delta))),
        LocalAttachmentEvent::ResumeDelta(delta) => Ok(Some(TerminalViewEvent::ResumeDelta(delta))),
        LocalAttachmentEvent::HistoryWindow(result) => {
            Ok(Some(TerminalViewEvent::HistoryWindow(result)))
        }
        LocalAttachmentEvent::ConnectionStatus(status) => {
            if route != TerminalViewRoute::Remote {
                return Err(terminal_protocol_error(
                    "local terminal received remote connection status",
                ));
            }
            let path = match zterm_proto::v2::TerminalConnectionPath::try_from(status.path)
                .map_err(|_| terminal_protocol_error("unknown terminal connection path"))?
            {
                zterm_proto::v2::TerminalConnectionPath::Unknown => {
                    TerminalViewConnectionPath::Unknown
                }
                zterm_proto::v2::TerminalConnectionPath::Direct => {
                    TerminalViewConnectionPath::Direct
                }
                zterm_proto::v2::TerminalConnectionPath::Relay => TerminalViewConnectionPath::Relay,
                zterm_proto::v2::TerminalConnectionPath::Unspecified => {
                    return Err(terminal_protocol_error(
                        "terminal connection path was unspecified",
                    ));
                }
            };
            Ok(Some(TerminalViewEvent::ConnectionStatus(
                TerminalViewConnectionStatus {
                    path,
                    rtt_ms: status.rtt_ms,
                },
            )))
        }
        LocalAttachmentEvent::SyncRequired(required) => Ok(Some(TerminalViewEvent::SyncRequired {
            latest_revision: Revision::new(required.latest_revision),
        })),
        LocalAttachmentEvent::LeaseLost(lost) => Ok(Some(TerminalViewEvent::LeaseLost {
            generation: lost.generation,
        })),
        LocalAttachmentEvent::SessionEnded(ended) => {
            let reason = match zterm_proto::v2::TerminalSessionEndReason::try_from(ended.reason)
                .map_err(|_| terminal_protocol_error("unknown terminal session end reason"))?
            {
                zterm_proto::v2::TerminalSessionEndReason::NaturalExit => {
                    TerminalViewEndReason::NaturalExit
                }
                zterm_proto::v2::TerminalSessionEndReason::ExplicitClose => {
                    TerminalViewEndReason::ExplicitClose
                }
                zterm_proto::v2::TerminalSessionEndReason::DaemonStop => {
                    TerminalViewEndReason::DaemonStop
                }
                zterm_proto::v2::TerminalSessionEndReason::DriverFailure => {
                    TerminalViewEndReason::DriverFailure
                }
                zterm_proto::v2::TerminalSessionEndReason::Unspecified => {
                    return Err(terminal_protocol_error(
                        "terminal session end reason was unspecified",
                    ));
                }
            };
            Ok(Some(TerminalViewEvent::SessionEnded(TerminalViewEnded {
                reason,
                exit_code: ended.exit_code,
                signal: ended.signal,
            })))
        }
        LocalAttachmentEvent::TransportState(_)
        | LocalAttachmentEvent::Takeover(_)
        | LocalAttachmentEvent::ClipboardWrite(_) => Ok(None),
    }
}

#[cfg(unix)]
fn terminal_transport_state_from_wire(
    value: i32,
) -> Result<TerminalViewTransportState, DaemonError> {
    match zterm_proto::v2::TerminalTransportState::try_from(value)
        .map_err(|_| terminal_protocol_error("unknown terminal transport state"))?
    {
        zterm_proto::v2::TerminalTransportState::Preparing => {
            Ok(TerminalViewTransportState::Preparing)
        }
        zterm_proto::v2::TerminalTransportState::Synchronizing => {
            Ok(TerminalViewTransportState::Synchronizing)
        }
        zterm_proto::v2::TerminalTransportState::Active => Ok(TerminalViewTransportState::Active),
        zterm_proto::v2::TerminalTransportState::Reconnecting => {
            Ok(TerminalViewTransportState::Reconnecting)
        }
        zterm_proto::v2::TerminalTransportState::Unspecified => Err(terminal_protocol_error(
            "terminal transport state was unspecified",
        )),
    }
}

#[cfg(unix)]
fn terminal_protocol_error(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::device_directory::ResolvedSessionTarget;
    #[cfg(unix)]
    use std::collections::VecDeque;
    #[cfg(unix)]
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[cfg(unix)]
    use zterm_core::DeviceId;
    #[cfg(unix)]
    use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v2};

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn full_command_queue_expires_and_dequeued_expired_input_is_not_written() {
        let (sender, mut queue) = tokio::sync::mpsc::channel(1);
        let (_outcome_sender, terminal_outcome_queued) = tokio::sync::watch::channel(false);
        let writer = TerminalViewCommandWriter {
            sender,
            terminal_outcome_queued,
            applied_revision: Arc::new(AtomicU64::new(1)),
        };
        let (response, received) = tokio::sync::oneshot::channel();
        writer
            .sender
            .send(PendingTerminalCommand {
                command: TerminalDriverCommand::Input {
                    bytes: b"expired".to_vec(),
                },
                deadline: super::super::control_deadline(),
                response,
            })
            .await
            .expect("fill single slot");
        let started = tokio::time::Instant::now();
        assert_eq!(
            writer
                .write_input(b"never admitted".to_vec())
                .await
                .expect_err("queue admission expires")
                .kind(),
            DomainErrorKind::DeadlineExceeded
        );
        assert_eq!(
            tokio::time::Instant::now() - started,
            super::super::DEFAULT_DEADLINE
        );

        let (mut client, peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            SessionId::from_array([1; 16]),
            AttachmentId::from_array([2; 16]),
        );
        assert!(matches!(
            handle_terminal_driver_command(queue.recv().await, &mut client).await,
            TerminalDriverCommandResult::Continue
        ));
        assert_eq!(
            received
                .await
                .expect("expired command completes")
                .expect_err("expired before starting")
                .kind(),
            DomainErrorKind::DeadlineExceeded
        );
        assert!(
            queue.try_recv().is_err(),
            "timed-out queue admission retained no command"
        );
        assert_eq!(
            peer.try_read(&mut [0_u8; 1])
                .expect_err("expired input never reached the transport")
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn command_write_uses_remaining_queue_budget_and_releases_the_owner() {
        let (mut client, mut peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            SessionId::from_array([1; 16]),
            AttachmentId::from_array([2; 16]),
        );
        let deadline = super::super::control_deadline();
        tokio::time::advance(Duration::from_secs(4)).await;
        let (response, received) = tokio::sync::oneshot::channel();
        let result = handle_terminal_driver_command(
            Some(PendingTerminalCommand {
                command: TerminalDriverCommand::Input {
                    bytes: vec![b'x'; 900_000],
                },
                deadline,
                response,
            }),
            &mut client,
        )
        .await;
        assert!(matches!(result, TerminalDriverCommandResult::Stop));
        assert_eq!(
            tokio::time::Instant::now(),
            deadline,
            "write cannot start a fresh five-second budget"
        );
        assert_eq!(
            received
                .await
                .expect("command result")
                .expect_err("blocked write expired")
                .kind(),
            DomainErrorKind::DeadlineExceeded
        );
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes)
            .await
            .expect("expired owner closes stream");
        assert!(bytes.len() < 900_000);
    }
    #[test]
    fn terminal_end_debug_redacts_the_platform_signal_text() {
        let ended = TerminalViewEnded {
            reason: TerminalViewEndReason::NaturalExit,
            exit_code: 1,
            signal: "SENSITIVE_TERMINAL_SIGNAL_SENTINEL".to_owned(),
        };
        let debug = format!("{ended:?}");
        assert!(debug.contains("has_signal: true"));
        assert!(!debug.contains("SENSITIVE_TERMINAL_SIGNAL_SENTINEL"));
    }

    #[cfg(unix)]
    #[test]
    fn connection_status_projection_is_route_explicit_and_redacts_private_identity() {
        const ATTACHMENT_SENTINEL: &[u8; AttachmentId::LENGTH] = b"STATUS_ID_SENTIN";
        let attachment_id = AttachmentId::from_array(*ATTACHMENT_SENTINEL);
        let project = |path, rtt_ms| {
            terminal_event_from_local(
                LocalAttachmentEvent::ConnectionStatus(v2::TerminalConnectionStatusEvent {
                    attachment_id: Some(attachment_id.into()),
                    path,
                    rtt_ms,
                }),
                TerminalViewRoute::Remote,
            )
            .expect("same-UID status projects")
            .expect("same-UID status remains visible")
        };

        for (event, expected_path, expected_rtt) in [
            (
                project(v2::TerminalConnectionPath::Unknown as i32, None),
                TerminalViewConnectionPath::Unknown,
                None,
            ),
            (
                project(v2::TerminalConnectionPath::Direct as i32, Some(7)),
                TerminalViewConnectionPath::Direct,
                Some(7),
            ),
            (
                project(v2::TerminalConnectionPath::Relay as i32, Some(19)),
                TerminalViewConnectionPath::Relay,
                Some(19),
            ),
        ] {
            let TerminalViewEvent::ConnectionStatus(status) = event else {
                panic!("connection status projects to its typed view event");
            };
            assert_eq!(status.path(), expected_path);
            assert_eq!(status.rtt_ms(), expected_rtt);
            let rendered = format!("{status:?}");
            assert!(!rendered.contains(
                std::str::from_utf8(ATTACHMENT_SENTINEL).expect("ASCII attachment sentinel")
            ));
        }

        let error = terminal_event_from_local(
            LocalAttachmentEvent::ConnectionStatus(v2::TerminalConnectionStatusEvent {
                attachment_id: Some(attachment_id.into()),
                path: v2::TerminalConnectionPath::Direct as i32,
                rtt_ms: Some(7),
            }),
            TerminalViewRoute::Local,
        )
        .expect_err("local-only views cannot accept remote connection status");
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_command_closure_prefers_typed_end_and_normalizes_plain_eof() {
        let session_id = SessionId::from_array([0xc1; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0xc2; AttachmentId::LENGTH]);
        let (mut client, mut peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        peer.write_all(
            &encode_message(
                WireKind::TerminalSessionEnded,
                0,
                0,
                &v2::TerminalSessionEnded {
                    session_id: Some(session_id.into()),
                    attachment_id: Some(attachment_id.into()),
                    reason: v2::TerminalSessionEndReason::NaturalExit as i32,
                    exit_code: 0,
                    signal: String::new(),
                },
            )
            .expect("encode typed terminal end"),
        )
        .await
        .expect("queue typed terminal end before command closure");
        peer.flush().await.expect("flush typed terminal end");

        let mut pending = VecDeque::new();
        let mut takeover_pending = false;
        let mut last_state = TerminalViewTransportState::Active;
        let (clipboard, _clipboard_wakeup) = TerminalClipboardSlot::new();
        let (response, received) = tokio::sync::oneshot::channel();
        correlate_terminal_command_closure(
            &mut client,
            &mut pending,
            TerminalViewRoute::Local,
            &mut takeover_pending,
            &mut last_state,
            response,
            &clipboard,
        )
        .await;
        assert_eq!(received.await.expect("command response owner"), Ok(()));
        assert!(matches!(
            pending.pop_front(),
            Some(Ok(TerminalViewEvent::SessionEnded(TerminalViewEnded {
                reason: TerminalViewEndReason::NaturalExit,
                ..
            })))
        ));
        assert!(pending.is_empty());

        let (mut eof_client, eof_peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        drop(eof_peer);
        let mut eof_pending = VecDeque::new();
        let (response, received) = tokio::sync::oneshot::channel();
        correlate_terminal_command_closure(
            &mut eof_client,
            &mut eof_pending,
            TerminalViewRoute::Local,
            &mut takeover_pending,
            &mut last_state,
            response,
            &clipboard,
        )
        .await;
        assert_eq!(received.await.expect("EOF command response owner"), Ok(()));
        let Some(Err(error)) = eof_pending.pop_front() else {
            panic!("plain attachment EOF must become one normalized typed error");
        };
        assert_eq!(error.kind(), DomainErrorKind::DaemonStopped);
        assert!(!error.detail().contains("Broken pipe"));
        assert!(!error.detail().contains("os error"));
        assert!(eof_pending.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_events_bypass_the_bounded_semantic_queue_and_keep_only_latest() {
        let (clipboard, mut wakeup) = TerminalClipboardSlot::new();
        let mut pending = VecDeque::from_iter((0..TERMINAL_DRIVER_CAPACITY).map(|_| {
            Ok(TerminalViewEvent::TransportState(
                TerminalViewTransportState::Active,
            ))
        }));
        let mut takeover_pending = false;
        let mut last_state = TerminalViewTransportState::Active;
        let first =
            TerminalClipboardWrite::new("first".to_owned()).expect("valid first clipboard value");
        let latest =
            TerminalClipboardWrite::new("latest".to_owned()).expect("valid latest clipboard value");

        assert!(!queue_local_attachment_event(
            Ok(LocalAttachmentEvent::ClipboardWrite(first)),
            &mut pending,
            TerminalViewRoute::Local,
            &mut takeover_pending,
            &mut last_state,
            &clipboard,
        ));
        assert!(!queue_local_attachment_event(
            Ok(LocalAttachmentEvent::ClipboardWrite(latest)),
            &mut pending,
            TerminalViewRoute::Local,
            &mut takeover_pending,
            &mut last_state,
            &clipboard,
        ));
        assert_eq!(pending.len(), TERMINAL_DRIVER_CAPACITY);
        assert!(wakeup.has_changed().expect("open clipboard wakeup"));
        wakeup.borrow_and_update();
        assert_eq!(
            clipboard
                .take()
                .expect("latest clipboard value remains")
                .as_str(),
            "latest"
        );
        assert!(clipboard.take().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_slot_is_cleared_by_reconnect_and_malformed_transport_state() {
        let attachment_id = AttachmentId::from_array([0xd2; AttachmentId::LENGTH]);

        for (state, terminal) in [
            (v2::TerminalTransportState::Reconnecting as i32, false),
            (i32::MAX, true),
        ] {
            let (clipboard, _clipboard_wakeup) = TerminalClipboardSlot::new();
            clipboard.replace(
                TerminalClipboardWrite::new("stale epoch clipboard".to_owned())
                    .expect("valid clipboard fixture"),
            );
            let mut pending = VecDeque::new();
            let mut takeover_pending = false;
            let mut last_state = TerminalViewTransportState::Active;

            assert_eq!(
                queue_local_attachment_event(
                    Ok(LocalAttachmentEvent::TransportState(
                        v2::TerminalTransportStateEvent {
                            attachment_id: Some(attachment_id.into()),
                            state,
                        },
                    )),
                    &mut pending,
                    TerminalViewRoute::Local,
                    &mut takeover_pending,
                    &mut last_state,
                    &clipboard,
                ),
                terminal,
            );
            assert!(
                clipboard.take().is_none(),
                "an epoch boundary or terminal protocol error must discard pending clipboard content"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn legal_resize_after_terminal_event_was_queued_preserves_the_typed_outcome() {
        let session_id = SessionId::from_array([0xc5; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0xc6; AttachmentId::LENGTH]);
        let (client, mut peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        let view = spawn_terminal_driver(
            client,
            TerminalViewTransportState::Active,
            TerminalViewRoute::Local,
            false,
        );
        let (mut events, writer) = view.split();
        assert!(matches!(
            events.read_event().await.expect("initial terminal event"),
            Some(TerminalViewEvent::TransportState(
                TerminalViewTransportState::Active
            ))
        ));

        peer.write_all(
            &encode_message(
                WireKind::TerminalSessionEnded,
                0,
                0,
                &v2::TerminalSessionEnded {
                    session_id: Some(session_id.into()),
                    attachment_id: Some(attachment_id.into()),
                    reason: v2::TerminalSessionEndReason::NaturalExit as i32,
                    exit_code: 0,
                    signal: String::new(),
                },
            )
            .expect("encode queued terminal outcome"),
        )
        .await
        .expect("queue terminal outcome");
        peer.flush().await.expect("flush terminal outcome");
        drop(peer);

        tokio::time::timeout(Duration::from_secs(1), writer.sender.closed())
            .await
            .expect("terminal driver must close its command owner after queuing the outcome");
        writer
            .resize(TerminalSize::new(31, 97))
            .await
            .expect("a legal resize must defer to the already queued terminal outcome");
        assert!(matches!(
            events
                .read_event()
                .await
                .expect("authoritative terminal event"),
            Some(TerminalViewEvent::SessionEnded(TerminalViewEnded {
                reason: TerminalViewEndReason::NaturalExit,
                ..
            }))
        ));

        let (command_sender, mut command_receiver) =
            tokio::sync::mpsc::channel(TERMINAL_DRIVER_CAPACITY);
        let (outcome_sender, outcome_receiver) = tokio::sync::watch::channel(false);
        let (event_sender, mut event_receiver) = tokio::sync::mpsc::channel::<TerminalViewEvent>(1);
        let response_owner = tokio::spawn(async move {
            let command = command_receiver
                .recv()
                .await
                .expect("receive the accepted resize command");
            event_sender
                .send(TerminalViewEvent::LeaseLost { generation: 41 })
                .await
                .expect("queue authoritative response-owner outcome");
            outcome_sender.send_replace(true);
            drop(command);
        });
        let response_closed_writer = TerminalViewCommandWriter {
            sender: command_sender,
            terminal_outcome_queued: outcome_receiver,
            applied_revision: Arc::new(AtomicU64::new(0)),
        };
        response_closed_writer
            .resize(TerminalSize::new(31, 97))
            .await
            .expect("a closed response owner must use the same event-side correlation");
        response_owner.await.expect("response-owner schedule");
        assert!(matches!(
            event_receiver.recv().await,
            Some(TerminalViewEvent::LeaseLost { generation: 41 })
        ));

        let (closed_sender, closed_receiver) = tokio::sync::mpsc::channel(TERMINAL_DRIVER_CAPACITY);
        drop(closed_receiver);
        let (unavailable_sender, unavailable_receiver) = tokio::sync::watch::channel(false);
        drop(unavailable_sender);
        let unavailable_writer = TerminalViewCommandWriter {
            sender: closed_sender,
            terminal_outcome_queued: unavailable_receiver,
            applied_revision: Arc::new(AtomicU64::new(0)),
        };
        let error = unavailable_writer
            .resize(TerminalSize::new(31, 97))
            .await
            .expect_err("closure without an authoritative event must stay a bounded error");
        assert_eq!(error.kind(), DomainErrorKind::DaemonStopped);
        assert!(!error.detail().contains("terminal attachment driver closed"));
        assert!(!error.detail().contains("Broken pipe"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropped_terminal_event_reader_never_confirms_a_closed_command() {
        let session_id = SessionId::from_array([0xc7; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0xc8; AttachmentId::LENGTH]);
        let (client, peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        let view = spawn_terminal_driver(
            client,
            TerminalViewTransportState::Active,
            TerminalViewRoute::Local,
            false,
        );
        let (events, writer) = view.split();
        drop(events);

        tokio::time::timeout(Duration::from_secs(1), writer.sender.closed())
            .await
            .expect("dropping the event owner must stop the terminal driver");
        let error = writer
            .resize(TerminalSize::new(31, 97))
            .await
            .expect_err("no command may be confirmed after its event owner was dropped");
        assert_eq!(error.kind(), DomainErrorKind::DaemonStopped);
        assert!(!error.detail().contains("terminal attachment driver closed"));
        assert!(!error.detail().contains("Broken pipe"));
        drop(peer);
    }

    #[cfg(unix)]
    async fn closed_resize_schedule(
        session_id: SessionId,
        attachment_id: AttachmentId,
        frames: impl IntoIterator<Item = Vec<u8>>,
        route: TerminalViewRoute,
    ) -> (
        VecDeque<Result<TerminalViewEvent, DaemonError>>,
        TerminalViewTransportState,
    ) {
        let (mut client, mut peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        for frame in frames {
            peer.write_all(&frame)
                .await
                .expect("queue closure-schedule frame");
        }
        peer.flush().await.expect("flush closure-schedule frames");
        drop(peer);

        let mut pending = VecDeque::new();
        let mut takeover_pending = false;
        let mut last_state = TerminalViewTransportState::Active;
        let mut stop_after_pending = false;
        let (clipboard, _clipboard_wakeup) = TerminalClipboardSlot::new();
        let (response, received) = tokio::sync::oneshot::channel();
        assert!(
            !apply_terminal_driver_command(
                Some(PendingTerminalCommand {
                    command: TerminalDriverCommand::Resize {
                        size: TerminalSize::new(31, 97)
                    },
                    deadline: super::super::control_deadline(),
                    response
                }),
                &mut client,
                &mut pending,
                route,
                &mut takeover_pending,
                &mut last_state,
                &mut stop_after_pending,
                &clipboard,
            )
            .await,
            "a correlated closure drains its typed outcome before the driver stops"
        );
        assert!(stop_after_pending);
        assert_eq!(received.await.expect("resize response owner"), Ok(()));
        (pending, last_state)
    }

    #[cfg(unix)]
    fn closure_schedule_frame<Message: prost::Message>(
        kind: WireKind,
        message: &Message,
    ) -> Vec<u8> {
        encode_message(kind, 0, 0, message).expect("encode closure-schedule frame")
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn resize_closure_schedules_preserve_typed_end_lease_error_and_remote_resync_order() {
        let session_id = SessionId::from_array([0xc3; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0xc4; AttachmentId::LENGTH]);

        let (mut ended, _) = closed_resize_schedule(
            session_id,
            attachment_id,
            [closure_schedule_frame(
                WireKind::TerminalSessionEnded,
                &v2::TerminalSessionEnded {
                    session_id: Some(session_id.into()),
                    attachment_id: Some(attachment_id.into()),
                    reason: v2::TerminalSessionEndReason::DaemonStop as i32,
                    exit_code: 0,
                    signal: String::new(),
                },
            )],
            TerminalViewRoute::Local,
        )
        .await;
        assert!(matches!(
            ended.pop_front(),
            Some(Ok(TerminalViewEvent::SessionEnded(TerminalViewEnded {
                reason: TerminalViewEndReason::DaemonStop,
                ..
            })))
        ));
        assert!(ended.is_empty());

        let (mut lost, _) = closed_resize_schedule(
            session_id,
            attachment_id,
            [closure_schedule_frame(
                WireKind::TerminalLeaseLost,
                &v2::TerminalLeaseLost {
                    attachment_id: Some(attachment_id.into()),
                    generation: 23,
                },
            )],
            TerminalViewRoute::Local,
        )
        .await;
        assert!(matches!(
            lost.pop_front(),
            Some(Ok(TerminalViewEvent::LeaseLost { generation: 23 }))
        ));
        assert!(lost.is_empty());

        let (mut resync, last_state) = closed_resize_schedule(
            session_id,
            attachment_id,
            [
                closure_schedule_frame(
                    WireKind::TerminalTransportStateEvent,
                    &v2::TerminalTransportStateEvent {
                        attachment_id: Some(attachment_id.into()),
                        state: v2::TerminalTransportState::Reconnecting as i32,
                    },
                ),
                closure_schedule_frame(
                    WireKind::TerminalConnectionStatusEvent,
                    &v2::TerminalConnectionStatusEvent {
                        attachment_id: Some(attachment_id.into()),
                        path: v2::TerminalConnectionPath::Unknown as i32,
                        rtt_ms: None,
                    },
                ),
                closure_schedule_frame(
                    WireKind::TerminalSyncRequired,
                    &v2::TerminalSyncRequired {
                        attachment_id: Some(attachment_id.into()),
                        latest_revision: 29,
                    },
                ),
            ],
            TerminalViewRoute::Remote,
        )
        .await;
        assert!(matches!(
            resync.pop_front(),
            Some(Ok(TerminalViewEvent::TransportState(
                TerminalViewTransportState::Reconnecting
            )))
        ));
        assert!(matches!(
            resync.pop_front(),
            Some(Ok(TerminalViewEvent::ConnectionStatus(_)))
        ));
        assert!(matches!(
            resync.pop_front(),
            Some(Ok(TerminalViewEvent::TransportState(
                TerminalViewTransportState::Synchronizing
            )))
        ));
        assert!(matches!(
            resync.pop_front(),
            Some(Ok(TerminalViewEvent::SyncRequired {
                latest_revision
            })) if latest_revision == Revision::new(29)
        ));
        let Some(Err(resync_closed)) = resync.pop_front() else {
            panic!("an incomplete resync followed by EOF is normalized once");
        };
        assert_eq!(resync_closed.kind(), DomainErrorKind::DaemonStopped);
        assert!(!resync_closed.detail().contains("Broken pipe"));
        assert!(resync.is_empty());
        assert_eq!(last_state, TerminalViewTransportState::Synchronizing);

        let typed_error = DaemonError::new(
            DomainErrorKind::Unauthorized,
            "BUFFERED_TYPED_ERROR_SENTINEL",
        );
        let (mut failed, _) = closed_resize_schedule(
            session_id,
            attachment_id,
            [closure_schedule_frame(
                WireKind::ServiceErrorResponse,
                &v2::ServiceError {
                    code: typed_error.kind().code().to_owned(),
                    message: typed_error.detail().to_owned(),
                },
            )],
            TerminalViewRoute::Local,
        )
        .await;
        let Some(Err(error)) = failed.pop_front() else {
            panic!("buffered typed service error wins the resize closure race");
        };
        assert_eq!(error.kind(), DomainErrorKind::Unauthorized);
        assert_eq!(error.detail(), "BUFFERED_TYPED_ERROR_SENTINEL");
        assert!(!error.detail().contains("Broken pipe"));
        assert!(failed.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_driver_local_exact_ack_activates_once_with_bounded_channels() {
        let (prepared, mut daemon, _, attachment_id) = terminal_test_view(false);
        let server = tokio::spawn(async move {
            let mut decoder = FrameDecoder::new();
            let mut queued = VecDeque::new();
            let initial_ack =
                read_terminal_test_frame(&mut daemon, &mut decoder, &mut queued).await;
            assert_eq!(initial_ack.kind, WireKind::TerminalSnapshotApplied);
            let initial: v2::TerminalSnapshotApplied = initial_ack
                .decode_message(WireKind::TerminalSnapshotApplied)
                .expect("initial snapshot acknowledgement");
            assert_eq!(initial.revision, 1);

            let repeated_ack =
                read_terminal_test_frame(&mut daemon, &mut decoder, &mut queued).await;
            assert_eq!(repeated_ack.kind, WireKind::TerminalSnapshotApplied);
            daemon
                .write_all(
                    &encode_message(
                        WireKind::TerminalLeaseLost,
                        0,
                        0,
                        &v2::TerminalLeaseLost {
                            attachment_id: Some(attachment_id.into()),
                            generation: 7,
                        },
                    )
                    .expect("bounded lease-lost marker"),
                )
                .await
                .expect("write lease-lost marker");

            let mut after_terminal = [0_u8; 1];
            assert_eq!(
                daemon
                    .read(&mut after_terminal)
                    .await
                    .expect("read the driver's terminal-event shutdown"),
                0,
                "the typed lease-loss event closes the local driver"
            );
        });

        let view = prepared
            .acknowledge_initial()
            .await
            .expect("exact initial acknowledgement");
        let (mut events, writer) = view.split();
        assert_eq!(writer.sender.max_capacity(), TERMINAL_DRIVER_CAPACITY);
        assert!(matches!(
            events.read_event().await.expect("initial event"),
            Some(TerminalViewEvent::TransportState(
                TerminalViewTransportState::Active
            ))
        ));
        writer
            .snapshot_applied(Revision::new(2))
            .await
            .expect("repeated acknowledgement is serialized");
        assert!(matches!(
            events.read_event().await.expect("post-ack marker"),
            Some(TerminalViewEvent::LeaseLost { generation: 7 })
        ));
        drop(writer);
        server.await.expect("local driver server");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn queued_resume_barrier_survives_view_state_transitions() {
        let (prepared, _peer, _, _) = terminal_test_view(false);
        let snapshot = prepared.initial_snapshot();
        let delta = TerminalViewDelta {
            from_revision: snapshot.revision,
            to_revision: Revision::new(snapshot.revision.get() + 1),
            size: snapshot.surface.size,
            active_screen: snapshot.surface.active_screen,
            row_patches: Vec::new(),
            cursor: snapshot.surface.cursor,
            modes: snapshot.surface.modes,
            scroll_metrics: None,
        };
        for route in [TerminalViewRoute::Local, TerminalViewRoute::Remote] {
            for initial in [
                TerminalViewTransportState::Active,
                TerminalViewTransportState::Synchronizing,
            ] {
                let mut pending = VecDeque::new();
                let mut takeover_pending = false;
                let mut state = initial;
                let (clipboard, _wakeup) = TerminalClipboardSlot::new();
                for event in [
                    LocalAttachmentEvent::Delta(delta.clone()),
                    LocalAttachmentEvent::ResumeDelta(delta.clone()),
                ] {
                    assert!(!queue_local_attachment_event(
                        Ok(event),
                        &mut pending,
                        route,
                        &mut takeover_pending,
                        &mut state,
                        &clipboard
                    ));
                }
                assert!(matches!(
                    pending.pop_front(),
                    Some(Ok(TerminalViewEvent::Delta(_)))
                ));
                if initial == TerminalViewTransportState::Active {
                    assert!(matches!(
                        pending.pop_front(),
                        Some(Ok(TerminalViewEvent::TransportState(
                            TerminalViewTransportState::Synchronizing
                        )))
                    ));
                }
                assert!(matches!(
                    pending.pop_front(),
                    Some(Ok(TerminalViewEvent::ResumeDelta(_)))
                ));
                assert!(pending.is_empty());
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lost_remote_snapshot_ack_stays_synchronizing_until_resume() {
        let target = DeviceId::from_array([0xd1; DeviceId::LENGTH]);
        let (mut client, mut dead_peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(target),
            SessionId::from_array([0xd2; SessionId::LENGTH]),
            AttachmentId::from_array([0xd3; AttachmentId::LENGTH]),
        );
        dead_peer
            .shutdown()
            .await
            .expect("close the simulated remote epoch");
        drop(dead_peer);

        let mut pending = VecDeque::new();
        let mut takeover_pending = false;
        let mut last_state = TerminalViewTransportState::Synchronizing;
        let mut stop_after_pending = false;
        let (clipboard, _clipboard_wakeup) = TerminalClipboardSlot::new();
        let (response, received) = tokio::sync::oneshot::channel();
        assert!(
            !apply_terminal_driver_command(
                Some(PendingTerminalCommand {
                    command: TerminalDriverCommand::SnapshotApplied {
                        revision: Revision::new(2)
                    },
                    deadline: super::super::control_deadline(),
                    response
                }),
                &mut client,
                &mut pending,
                TerminalViewRoute::Remote,
                &mut takeover_pending,
                &mut last_state,
                &mut stop_after_pending,
                &clipboard,
            )
            .await
        );
        assert_eq!(received.await.expect("snapshot ack response owner"), Ok(()));
        assert!(client.reconnect_pending());
        assert_eq!(last_state, TerminalViewTransportState::Synchronizing);
        assert!(pending.is_empty(), "a dead epoch must not emit Active");
        assert!(!stop_after_pending);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lost_initial_remote_snapshot_ack_starts_driver_reconnecting_not_active() {
        let target_device = DeviceId::from_array([0xd4; DeviceId::LENGTH]);
        let session_id = SessionId::from_array([0xd5; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0xd6; AttachmentId::LENGTH]);
        let (mut client, mut dead_peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(target_device),
            session_id,
            attachment_id,
        );
        let initial_snapshot = client
            .take_initial_snapshot()
            .expect("new attachment retains its initial snapshot");
        dead_peer
            .shutdown()
            .await
            .expect("close the simulated initial remote epoch");
        drop(dead_peer);
        let prepared = PreparedTerminalView {
            session_id,
            attachment_id,
            initial_snapshot,
            takeover: false,
            target: TerminalViewTarget::for_display("peer", TerminalViewRoute::Remote),
            client,
        };

        let view = prepared
            .acknowledge_initial()
            .await
            .expect("lost initial ack enters recovery instead of failing locally");
        let (mut events, writer) = view.split();
        assert!(matches!(
            events
                .read_event()
                .await
                .expect("initial reconnecting event"),
            Some(TerminalViewEvent::TransportState(
                TerminalViewTransportState::Reconnecting
            ))
        ));
        drop(events);
        drop(writer);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_driver_takeover_activates_only_after_correlated_response() {
        let (prepared, mut daemon, session_id, _) = terminal_test_view(true);
        let server = tokio::spawn(async move {
            let mut decoder = FrameDecoder::new();
            let mut queued = VecDeque::new();
            let ack = read_terminal_test_frame(&mut daemon, &mut decoder, &mut queued).await;
            assert_eq!(ack.kind, WireKind::TerminalSnapshotApplied);
            let lease_request =
                read_terminal_test_frame(&mut daemon, &mut decoder, &mut queued).await;
            assert_eq!(lease_request.kind, WireKind::SessionOperationLeaseRequest);
            daemon
                .write_all(
                    &encode_message(
                        WireKind::SessionOperationLeaseResponse,
                        lease_request.request_id,
                        0,
                        &test_operation_lease(),
                    )
                    .expect("bounded operation lease response"),
                )
                .await
                .expect("write operation lease response");
            let takeover = read_terminal_test_frame(&mut daemon, &mut decoder, &mut queued).await;
            assert_eq!(takeover.kind, WireKind::SessionTakeoverRequest);
            daemon
                .write_all(
                    &encode_message(
                        WireKind::SessionMutateResponse,
                        takeover.request_id,
                        0,
                        &test_takeover_response(session_id),
                    )
                    .expect("bounded correlated takeover response"),
                )
                .await
                .expect("write correlated takeover response");
            let detach = read_terminal_test_frame(&mut daemon, &mut decoder, &mut queued).await;
            assert_eq!(detach.kind, WireKind::TerminalDetach);
            let mut after_detach = [0_u8; 1];
            assert_eq!(
                daemon
                    .read(&mut after_detach)
                    .await
                    .expect("read the takeover driver's detach shutdown"),
                0,
                "the takeover driver half-closes after the detach frame"
            );
        });

        let view = prepared
            .acknowledge_initial()
            .await
            .expect("begin exact takeover");
        let (mut events, writer) = view.split();
        assert!(matches!(
            events.read_event().await.expect("initial takeover state"),
            Some(TerminalViewEvent::TransportState(
                TerminalViewTransportState::Synchronizing
            ))
        ));
        assert!(matches!(
            events
                .read_event()
                .await
                .expect("correlated takeover state"),
            Some(TerminalViewEvent::TransportState(
                TerminalViewTransportState::Active
            ))
        ));
        writer.detach().await.expect("detach takeover view");
        server.await.expect("takeover server");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_driver_outcome_unknown_is_terminal_and_never_replays_takeover() {
        let (prepared, mut daemon, _, _) = terminal_test_view(true);
        let server = tokio::spawn(async move {
            let mut decoder = FrameDecoder::new();
            let mut queued = VecDeque::new();
            let ack = read_terminal_test_frame(&mut daemon, &mut decoder, &mut queued).await;
            assert_eq!(ack.kind, WireKind::TerminalSnapshotApplied);
            let lease_request =
                read_terminal_test_frame(&mut daemon, &mut decoder, &mut queued).await;
            daemon
                .write_all(
                    &encode_message(
                        WireKind::SessionOperationLeaseResponse,
                        lease_request.request_id,
                        0,
                        &test_operation_lease(),
                    )
                    .expect("bounded operation lease response"),
                )
                .await
                .expect("write operation lease response");
            let takeover = read_terminal_test_frame(&mut daemon, &mut decoder, &mut queued).await;
            assert_eq!(takeover.kind, WireKind::SessionTakeoverRequest);
            daemon
                .write_all(
                    &encode_message(
                        WireKind::ServiceErrorResponse,
                        takeover.request_id,
                        0,
                        &v2::ServiceError {
                            code: DomainErrorKind::OperationOutcomeUnknown.code().to_owned(),
                            message: "takeover outcome is unknown".to_owned(),
                        },
                    )
                    .expect("bounded outcome-unknown response"),
                )
                .await
                .expect("write outcome-unknown response");
            let mut unexpected = [0_u8; 1];
            assert_eq!(
                daemon.read(&mut unexpected).await.expect("driver closes"),
                0,
                "the terminal driver must not retry under a fresh lease"
            );
        });

        let view = prepared
            .acknowledge_initial()
            .await
            .expect("begin takeover before ambiguous result");
        let (mut events, _writer) = view.split();
        assert!(matches!(
            events.read_event().await.expect("initial takeover state"),
            Some(TerminalViewEvent::TransportState(
                TerminalViewTransportState::Synchronizing
            ))
        ));
        let error = events
            .read_event()
            .await
            .expect_err("outcome unknown is a typed terminal driver error");
        assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
        server.await.expect("outcome-unknown server");
    }

    #[cfg(unix)]
    fn terminal_test_view(
        takeover: bool,
    ) -> (
        PreparedTerminalView,
        tokio::net::UnixStream,
        SessionId,
        AttachmentId,
    ) {
        let session_id = SessionId::from_array([0xb1; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0xb2; AttachmentId::LENGTH]);
        let (mut client, peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        let initial_snapshot = client
            .take_initial_snapshot()
            .expect("new attachment retains its initial snapshot");
        (
            PreparedTerminalView {
                session_id,
                attachment_id,
                initial_snapshot,
                takeover,
                target: TerminalViewTarget::for_display("local", TerminalViewRoute::Local),
                client,
            },
            peer,
            session_id,
            attachment_id,
        )
    }

    #[cfg(unix)]
    async fn read_terminal_test_frame(
        stream: &mut tokio::net::UnixStream,
        decoder: &mut FrameDecoder,
        queued: &mut VecDeque<DecodedFrame>,
    ) -> DecodedFrame {
        if let Some(frame) = queued.pop_front() {
            return frame;
        }
        let mut bytes = [0_u8; 4096];
        loop {
            let read = stream.read(&mut bytes).await.expect("read driver frame");
            assert_ne!(
                read, 0,
                "terminal driver stream closed before the next frame"
            );
            queued.extend(decoder.feed(&bytes[..read]).expect("decode driver frame"));
            if let Some(frame) = queued.pop_front() {
                return frame;
            }
        }
    }

    #[cfg(unix)]
    fn test_operation_lease() -> v2::SessionOperationLeaseResponse {
        v2::SessionOperationLeaseResponse {
            lease: Some(v2::OperationLease {
                daemon_incarnation: vec![0xb3; 16],
                ordinal: 1,
            }),
        }
    }

    #[cfg(unix)]
    fn test_takeover_response(session_id: SessionId) -> v2::SessionMutateResponse {
        v2::SessionMutateResponse {
            session: Some(v2::SessionSummary {
                session_id: Some(session_id.into()),
                name: "main".to_owned(),
                revision: 1,
                has_controller: true,
                working_directory: String::new(),
                viewport: Some(v2::TerminalViewport {
                    rows: 24,
                    columns: 80,
                }),
            }),
        }
    }
}
