//! Daemon-aware command backend shared by the thin CLI.

use std::fmt;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

use zterm_core::terminal::{
    ActiveScreen, TerminalHistoryCursor, TerminalHistoryDirection, TerminalHistoryResult,
    TerminalModes, TerminalScrollAction, TerminalScrollMetrics, TerminalSize,
    TerminalViewportResult,
};
#[cfg(unix)]
use zterm_core::terminal::{
    TerminalHistoryPage, TerminalMouseEncoding, TerminalMouseMode, TerminalViewportDisposition,
    TerminalViewportFrame,
};
use zterm_core::{
    AttachmentId, AuthGeneration, AuthorizationStatus, DeviceAlias, DeviceId, DeviceSummary,
    DomainErrorKind, Revision, SessionId, SessionName, SessionSelector,
};
use zterm_platform::user_state::UserPaths;

use crate::bootstrap::BootstrapResult;
#[cfg(unix)]
use crate::bootstrap::bootstrap_with_lock_held;
use crate::config::ValidatedConfig;
use crate::device_directory::ResolvedSessionTarget;
use crate::error::DaemonError;
#[cfg(unix)]
use crate::lifecycle::acquire_lifecycle_lock;
use crate::lifecycle::{DaemonLauncher, probe_readiness};
#[cfg(unix)]
use crate::local_ipc::{
    LocalAttachmentClient, LocalAttachmentEvent, LocalPairingClient,
    is_attachment_command_stream_closed, is_attachment_stream_closed_without_event,
};
use crate::local_ipc::{LocalClient, LocalDeviceClient};
use crate::pairing::PairTicketText;
use crate::service::{DaemonReadiness, DaemonStatus, SessionImpact};

const MAX_LOG_LINES: usize = 1_000;
const MAX_LOG_BYTES: u64 = 1024 * 1024;
const IDENTITY_RESET_STOP_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const TERMINAL_DRIVER_CAPACITY: usize = 8;
#[cfg(unix)]
const TERMINAL_CLOSURE_CORRELATION_WINDOW: Duration = Duration::from_millis(100);

/// Side-effect-free observation of local setup and daemon state.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum ObservedState {
    /// Setup is complete and the daemon answered status.
    Running(DaemonStatus),
    /// Setup is complete but no daemon is listening.
    ConfiguredStopped(BootstrapResult),
    /// No complete setup exists.
    NotConfigured,
}

/// One diagnostic check from the non-spawning doctor command.
#[derive(Clone, Eq, PartialEq)]
pub struct DoctorCheck {
    /// Stable check name.
    pub name: &'static str,
    /// Whether the check passed.
    pub ok: bool,
    /// Bounded human-readable detail.
    pub detail: String,
}

impl fmt::Debug for DoctorCheck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DoctorCheck")
            .field("name", &self.name)
            .field("ok", &self.ok)
            .field("detail", &"[REDACTED]")
            .field("detail_len", &self.detail.len())
            .finish()
    }
}

/// Non-spawning local diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoctorReport {
    /// True when every required local-state check passed.
    pub healthy: bool,
    /// Ordered diagnostic checks.
    pub checks: Vec<DoctorCheck>,
}

/// Route-free directional device projection exposed to public command clients.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandDeviceSummary {
    /// Exact full device identity.
    pub device_id: DeviceId,
    /// Whether this host has an outbound known-device record.
    pub outbound_known: bool,
    /// Exact outbound alias, when present.
    pub alias: Option<String>,
    /// Remote-provided display name in the outbound record.
    pub remote_name: Option<String>,
    /// Whether the remote may currently control this host.
    pub inbound_status: AuthorizationStatus,
    /// Inbound authorization generation.
    pub generation: AuthGeneration,
    /// Inbound pairing timestamp, or zero when no inbound record exists.
    pub paired_at_unix: u64,
    /// Last observed activity timestamp.
    pub last_seen_at_unix: u64,
    /// Whether a live primary connection exists.
    pub online: bool,
    /// Current bounded service-stream count.
    pub active_stream_count: u32,
    /// Current matching remote attachment count.
    pub remote_attachment_count: u32,
}

impl From<DeviceSummary> for CommandDeviceSummary {
    fn from(summary: DeviceSummary) -> Self {
        Self {
            device_id: summary.device_id(),
            outbound_known: summary.outbound_known(),
            alias: summary.alias().map(|alias| alias.as_str().to_owned()),
            remote_name: summary
                .outbound_known()
                .then(|| summary.remote_name().to_owned()),
            inbound_status: summary.auth_status(),
            generation: summary.generation(),
            paired_at_unix: summary.paired_at_unix(),
            last_seen_at_unix: summary.last_seen_at_unix(),
            online: summary.online(),
            active_stream_count: summary.active_stream_count(),
            remote_attachment_count: summary.remote_attachment_count(),
        }
    }
}

/// Path-free public projection of one live daemon-lifetime Session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSessionSummary {
    /// Stable daemon-lifetime Session identity.
    pub session_id: SessionId,
    /// Current exact Session name.
    pub name: SessionName,
    /// Latest host-authoritative terminal revision.
    pub revision: Revision,
    /// Whether a controller is attached.
    pub has_controller: bool,
    /// Last accepted viewport.
    pub viewport: TerminalSize,
}

impl From<crate::session::SessionSummary> for CommandSessionSummary {
    fn from(summary: crate::session::SessionSummary) -> Self {
        Self {
            session_id: summary.session_id,
            name: summary.name,
            revision: summary.revision,
            has_controller: summary.has_controller,
            viewport: summary.viewport,
        }
    }
}

/// Created Session plus its runtime-owned frozen exact target.
pub struct CreatedSession {
    target: ResolvedSessionTarget,
    summary: CommandSessionSummary,
}

impl fmt::Debug for CreatedSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreatedSession")
            .field("target", &self.target)
            .field("summary", &self.summary)
            .finish()
    }
}

impl CreatedSession {
    /// Safe public projection of the successfully created Session.
    #[must_use]
    pub const fn summary(&self) -> &CommandSessionSummary {
        &self.summary
    }
}

/// Side-effect-free close impact bound to one frozen target and Session ID.
pub struct SessionClosePreflight {
    target: ResolvedSessionTarget,
    summary: CommandSessionSummary,
}

impl fmt::Debug for SessionClosePreflight {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionClosePreflight")
            .field("target", &self.target)
            .field("summary", &self.summary)
            .finish()
    }
}

impl SessionClosePreflight {
    /// Safe public projection rendered before confirmation.
    #[must_use]
    pub const fn summary(&self) -> &CommandSessionSummary {
        &self.summary
    }

    /// Exact public target identity, or `None` for this local daemon.
    #[must_use]
    pub const fn target_device_id(&self) -> Option<DeviceId> {
        self.target.device_id()
    }
}

/// Daemon-authored replacement state consumed by the one CLI renderer.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalViewSnapshot {
    revision: Revision,
    size: TerminalSize,
    active_screen: ActiveScreen,
    modes: TerminalModes,
    recent_history_ansi: Vec<u8>,
    screen_ansi: Vec<u8>,
    scroll_metrics: Option<TerminalScrollMetrics>,
}

impl fmt::Debug for TerminalViewSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewSnapshot")
            .field("revision", &self.revision)
            .field("size", &self.size)
            .field("active_screen", &self.active_screen)
            .field("modes", &self.modes)
            .field("recent_history_ansi_len", &self.recent_history_ansi.len())
            .field("screen_ansi_len", &self.screen_ansi.len())
            .field("scroll_metrics", &self.scroll_metrics)
            .finish()
    }
}

impl TerminalViewSnapshot {
    /// Exact authoritative revision represented by this replacement.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Host viewport represented by this replacement.
    #[must_use]
    pub const fn size(&self) -> TerminalSize {
        self.size
    }

    /// Host screen selected after applying this replacement.
    #[must_use]
    pub const fn active_screen(&self) -> ActiveScreen {
        self.active_screen
    }

    /// Authoritative child input modes represented by this replacement.
    #[must_use]
    pub const fn modes(&self) -> TerminalModes {
        self.modes
    }

    /// Bounded recent main-screen history, applied before [`Self::screen_ansi`].
    #[must_use]
    pub fn recent_history_ansi(&self) -> &[u8] {
        &self.recent_history_ansi
    }

    /// Daemon-authored current-screen ANSI.
    #[must_use]
    pub fn screen_ansi(&self) -> &[u8] {
        &self.screen_ansi
    }

    /// Live main-screen scroll extent, absent for alternate or legacy peers.
    #[must_use]
    pub const fn scroll_metrics(&self) -> Option<TerminalScrollMetrics> {
        self.scroll_metrics
    }
}

/// Daemon-authored merged update from one exact acknowledged revision.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalViewDelta {
    from_revision: Revision,
    to_revision: Revision,
    size: TerminalSize,
    active_screen: ActiveScreen,
    modes: TerminalModes,
    ansi: Vec<u8>,
    scroll_metrics: Option<TerminalScrollMetrics>,
}

impl fmt::Debug for TerminalViewDelta {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewDelta")
            .field("from_revision", &self.from_revision)
            .field("to_revision", &self.to_revision)
            .field("size", &self.size)
            .field("active_screen", &self.active_screen)
            .field("modes", &self.modes)
            .field("ansi_len", &self.ansi.len())
            .field("scroll_metrics", &self.scroll_metrics)
            .finish()
    }
}

impl TerminalViewDelta {
    /// Revision against which this merged update was authored.
    #[must_use]
    pub const fn from_revision(&self) -> Revision {
        self.from_revision
    }

    /// Revision after this merged update is applied.
    #[must_use]
    pub const fn to_revision(&self) -> Revision {
        self.to_revision
    }

    /// Host viewport after this merged update.
    #[must_use]
    pub const fn size(&self) -> TerminalSize {
        self.size
    }

    /// Host screen selected after this merged update.
    #[must_use]
    pub const fn active_screen(&self) -> ActiveScreen {
        self.active_screen
    }

    /// Authoritative child input modes after this update.
    #[must_use]
    pub const fn modes(&self) -> TerminalModes {
        self.modes
    }

    /// Daemon-authored ANSI for this contiguous update.
    #[must_use]
    pub fn ansi(&self) -> &[u8] {
        &self.ansi
    }

    /// Live main-screen scroll extent, absent for alternate or legacy peers.
    #[must_use]
    pub const fn scroll_metrics(&self) -> Option<TerminalScrollMetrics> {
        self.scroll_metrics
    }
}

/// Monotonic desired-view transport state projected by the local daemon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalViewTransportState {
    /// The daemon is acquiring transport resources.
    Preparing,
    /// A full state or resume delta is awaiting exact acknowledgement.
    Synchronizing,
    /// Input and resize may be sent.
    Active,
    /// The daemon is replacing a lost remote stream.
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

/// Frozen remote device label plus current selected path and RTT.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalViewConnectionStatus {
    device: String,
    path: TerminalViewConnectionPath,
    rtt_ms: Option<u32>,
}

impl fmt::Debug for TerminalViewConnectionStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewConnectionStatus")
            .field("device", &"[REDACTED]")
            .field("device_len", &self.device.len())
            .field("path", &self.path)
            .field("rtt_ms", &self.rtt_ms)
            .finish()
    }
}

impl TerminalViewConnectionStatus {
    /// Frozen local alias for the exact remote device.
    #[must_use]
    pub fn device(&self) -> &str {
        &self.device
    }

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

/// One typed event from the daemon-owned attachment driver.
#[derive(Clone, Eq, PartialEq)]
pub enum TerminalViewEvent {
    /// Desired-view connectivity changed.
    TransportState(TerminalViewTransportState),
    /// Selected path and RTT changed for this remote view.
    ConnectionStatus(TerminalViewConnectionStatus),
    /// Replace the local rendered state atomically.
    Snapshot(TerminalViewSnapshot),
    /// Apply one merged update only when its baseline is contiguous.
    Delta(TerminalViewDelta),
    /// One correlated bounded page from daemon-authoritative history.
    History(TerminalHistoryResult),
    /// One correlated complete attachment-local semantic viewport outcome.
    Viewport(TerminalViewportResult),
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
            Self::History(history) => formatter.debug_tuple("History").field(history).finish(),
            Self::Viewport(viewport) => formatter.debug_tuple("Viewport").field(viewport).finish(),
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
    remote_alias: Option<String>,
    #[cfg(unix)]
    client: LocalAttachmentClient,
}

impl fmt::Debug for PreparedTerminalView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedTerminalView")
            .field("session_id", &self.session_id)
            .field("attachment_id", &self.attachment_id)
            .field("initial_revision", &self.initial_snapshot.revision)
            .field("takeover", &self.takeover)
            .field("remote", &self.remote_alias.is_some())
            .field("has_remote_alias", &self.remote_alias.is_some())
            .finish_non_exhaustive()
    }
}

impl PreparedTerminalView {
    /// Stable Session selected by this prepared view.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Stable local view identity.
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

    /// Frozen local alias rendered by the remote-only status row.
    #[must_use]
    pub fn remote_alias(&self) -> Option<&str> {
        self.remote_alias.as_deref()
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
        let initial_state = if self.remote_alias.is_some() || self.takeover {
            TerminalViewTransportState::Synchronizing
        } else {
            TerminalViewTransportState::Active
        };
        Ok(spawn_terminal_driver(
            self.client,
            initial_state,
            self.remote_alias,
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

/// Bounded event side of one daemon-owned terminal driver.
pub struct TerminalViewEventReader {
    #[cfg(unix)]
    receiver: tokio::sync::mpsc::Receiver<Result<TerminalViewEvent, DaemonError>>,
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
            self.receiver.recv().await.transpose()
        }
        #[cfg(not(unix))]
        {
            Err(unsupported_command_platform())
        }
    }
}

/// Cloneable typed command side of one daemon-owned terminal driver.
#[derive(Clone)]
pub struct TerminalViewCommandWriter {
    #[cfg(unix)]
    sender: tokio::sync::mpsc::Sender<TerminalDriverCommand>,
    #[cfg(unix)]
    terminal_outcome_queued: tokio::sync::watch::Receiver<bool>,
}

impl fmt::Debug for TerminalViewCommandWriter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewCommandWriter")
            .finish_non_exhaustive()
    }
}

impl TerminalViewCommandWriter {
    /// Acknowledges an exactly flushed replacement snapshot or resume delta.
    pub async fn snapshot_applied(&self, revision: Revision) -> Result<(), DaemonError> {
        #[cfg(unix)]
        {
            self.submit(|response| TerminalDriverCommand::SnapshotApplied { revision, response })
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
            self.submit(|response| TerminalDriverCommand::Input { bytes, response })
                .await
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
            self.submit(|response| TerminalDriverCommand::Resize { size, response })
                .await
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
            self.submit(|response| TerminalDriverCommand::RequestSync {
                known_revision,
                response,
            })
            .await
        }
        #[cfg(not(unix))]
        {
            let _ = known_revision;
            Err(unsupported_command_platform())
        }
    }

    /// Requests one bounded main-screen history page. The driver permits only
    /// one outstanding page request for this view.
    pub async fn request_history(
        &self,
        direction: TerminalHistoryDirection,
        cursor: Option<TerminalHistoryCursor>,
        maximum_rows: usize,
    ) -> Result<(), DaemonError> {
        #[cfg(unix)]
        {
            self.submit(|response| TerminalDriverCommand::RequestHistory {
                direction,
                cursor,
                maximum_rows,
                response,
            })
            .await
        }
        #[cfg(not(unix))]
        {
            let _ = (direction, cursor, maximum_rows);
            Err(unsupported_command_platform())
        }
    }

    /// Requests one attachment-local semantic viewport action.
    pub async fn request_viewport(&self, action: TerminalScrollAction) -> Result<(), DaemonError> {
        #[cfg(unix)]
        {
            self.submit(|response| TerminalDriverCommand::RequestViewport { action, response })
                .await
        }
        #[cfg(not(unix))]
        {
            let _ = action;
            Err(unsupported_command_platform())
        }
    }

    /// Detaches this view while leaving the Session and PTY running.
    pub async fn detach(&self) -> Result<(), DaemonError> {
        #[cfg(unix)]
        {
            self.submit(|response| TerminalDriverCommand::Detach { response })
                .await
        }
        #[cfg(not(unix))]
        {
            Err(unsupported_command_platform())
        }
    }

    #[cfg(unix)]
    async fn submit(
        &self,
        command: impl FnOnce(
            tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
        ) -> TerminalDriverCommand,
    ) -> Result<(), DaemonError> {
        let (response, receiver) = tokio::sync::oneshot::channel();
        if self.sender.send(command(response)).await.is_err() {
            return self.correlate_terminal_outcome().await;
        }
        match receiver.await {
            Ok(result) => result,
            Err(_) => self.correlate_terminal_outcome().await,
        }
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
enum TerminalDriverCommand {
    SnapshotApplied {
        revision: Revision,
        response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
    },
    Input {
        bytes: Vec<u8>,
        response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
    },
    Resize {
        size: TerminalSize,
        response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
    },
    RequestSync {
        known_revision: Revision,
        response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
    },
    RequestHistory {
        direction: TerminalHistoryDirection,
        cursor: Option<TerminalHistoryCursor>,
        maximum_rows: usize,
        response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
    },
    RequestViewport {
        action: TerminalScrollAction,
        response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
    },
    Detach {
        response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
    },
}

#[cfg(unix)]
fn spawn_terminal_driver(
    client: LocalAttachmentClient,
    initial_state: TerminalViewTransportState,
    remote_alias: Option<String>,
    takeover: bool,
) -> TerminalViewIo {
    let (command_sender, command_receiver) = tokio::sync::mpsc::channel(TERMINAL_DRIVER_CAPACITY);
    let (event_sender, event_receiver) = tokio::sync::mpsc::channel(TERMINAL_DRIVER_CAPACITY);
    let (terminal_outcome_sender, terminal_outcome_receiver) = tokio::sync::watch::channel(false);
    tokio::spawn(run_terminal_driver(
        client,
        command_receiver,
        event_sender,
        terminal_outcome_sender,
        initial_state,
        remote_alias,
        takeover,
    ));
    TerminalViewIo {
        reader: TerminalViewEventReader {
            receiver: event_receiver,
        },
        writer: TerminalViewCommandWriter {
            sender: command_sender,
            terminal_outcome_queued: terminal_outcome_receiver,
        },
    }
}

#[cfg(unix)]
async fn run_terminal_driver(
    mut client: LocalAttachmentClient,
    mut commands: tokio::sync::mpsc::Receiver<TerminalDriverCommand>,
    events: tokio::sync::mpsc::Sender<Result<TerminalViewEvent, DaemonError>>,
    terminal_outcome_queued: tokio::sync::watch::Sender<bool>,
    initial_state: TerminalViewTransportState,
    remote_alias: Option<String>,
    takeover: bool,
) {
    use std::collections::VecDeque;

    let remote = remote_alias.is_some();
    let mut pending = VecDeque::from([Ok(TerminalViewEvent::TransportState(initial_state))]);
    if let Some(device) = remote_alias.as_ref() {
        pending.push_back(Ok(TerminalViewEvent::ConnectionStatus(
            TerminalViewConnectionStatus {
                device: device.clone(),
                path: TerminalViewConnectionPath::Unknown,
                rtt_ms: None,
            },
        )));
    }
    let mut stop_after_pending = false;
    let mut local_takeover_pending = takeover && !remote;
    let mut last_state = initial_state;

    loop {
        if pending.is_empty() {
            tokio::select! {
                command = commands.recv() => {
                    if apply_terminal_driver_command(
                        command,
                        &mut client,
                        &mut pending,
                        remote_alias.as_deref(),
                        remote,
                        &mut local_takeover_pending,
                        &mut last_state,
                        &mut stop_after_pending,
                    ).await {
                        return;
                    }
                }
                () = events.closed() => {
                    let _ = client.detach().await;
                    return;
                }
                event = client.read_next_event() => {
                    if queue_local_attachment_event(
                        event,
                        &mut pending,
                        remote_alias.as_deref(),
                        &mut local_takeover_pending,
                        &mut last_state,
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
                    remote_alias.as_deref(),
                    remote,
                    &mut local_takeover_pending,
                    &mut last_state,
                    &mut stop_after_pending,
                ).await {
                    return;
                }
            }
            permit = events.reserve() => {
                let Ok(permit) = permit else {
                    let _ = client.detach().await;
                    return;
                };
                let event = pending.pop_front().expect("pending event was checked above");
                permit.send(event);
                if pending.is_empty() && stop_after_pending {
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
    command: Option<TerminalDriverCommand>,
    client: &mut LocalAttachmentClient,
    pending: &mut std::collections::VecDeque<Result<TerminalViewEvent, DaemonError>>,
    remote_alias: Option<&str>,
    remote: bool,
    local_takeover_pending: &mut bool,
    last_state: &mut TerminalViewTransportState,
    stop_after_pending: &mut bool,
) -> bool {
    match handle_terminal_driver_command(command, client).await {
        TerminalDriverCommandResult::Continue => false,
        TerminalDriverCommandResult::SnapshotApplied
            if !remote
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
                remote_alias,
                local_takeover_pending,
                last_state,
                response,
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
    client: &mut LocalAttachmentClient,
    pending: &mut std::collections::VecDeque<Result<TerminalViewEvent, DaemonError>>,
    remote_alias: Option<&str>,
    local_takeover_pending: &mut bool,
    last_state: &mut TerminalViewTransportState,
    response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
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
            remote_alias,
            local_takeover_pending,
            last_state,
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
    remote_alias: Option<&str>,
    local_takeover_pending: &mut bool,
    last_state: &mut TerminalViewTransportState,
) -> bool {
    match event {
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
                Ok(state) if state == *last_state => false,
                Ok(state) => {
                    *last_state = state;
                    pending.push_back(Ok(TerminalViewEvent::TransportState(state)));
                    false
                }
                Err(error) => {
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
            if local_event_requires_synchronizing(&event)
                && *last_state != TerminalViewTransportState::Synchronizing
            {
                *last_state = TerminalViewTransportState::Synchronizing;
                pending.push_back(Ok(TerminalViewEvent::TransportState(*last_state)));
            }
            match terminal_event_from_local(event, remote_alias) {
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
            pending.push_back(Err(error));
            true
        }
    }
}

#[cfg(unix)]
async fn handle_terminal_driver_command(
    command: Option<TerminalDriverCommand>,
    client: &mut LocalAttachmentClient,
) -> TerminalDriverCommandResult {
    let Some(command) = command else {
        let _ = client.detach().await;
        return TerminalDriverCommandResult::Stop;
    };
    let (result, response, success) = match command {
        TerminalDriverCommand::SnapshotApplied { revision, response } => (
            client.snapshot_applied(revision).await,
            response,
            TerminalDriverCommandResult::SnapshotApplied,
        ),
        TerminalDriverCommand::Input { bytes, response } => (
            client.write_input(bytes).await,
            response,
            TerminalDriverCommandResult::Continue,
        ),
        TerminalDriverCommand::Resize { size, response } => (
            client.resize(size).await,
            response,
            TerminalDriverCommandResult::Continue,
        ),
        TerminalDriverCommand::RequestSync {
            known_revision,
            response,
        } => (
            client.request_sync(known_revision).await,
            response,
            TerminalDriverCommandResult::Continue,
        ),
        TerminalDriverCommand::RequestHistory {
            direction,
            cursor,
            maximum_rows,
            response,
        } => (
            client
                .request_history(direction, cursor, maximum_rows)
                .await,
            response,
            TerminalDriverCommandResult::Continue,
        ),
        TerminalDriverCommand::RequestViewport { action, response } => (
            client.request_viewport(action).await,
            response,
            TerminalDriverCommandResult::Continue,
        ),
        TerminalDriverCommand::Detach { response } => (
            client.detach().await,
            response,
            TerminalDriverCommandResult::Stop,
        ),
    };
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
        LocalAttachmentEvent::Snapshot(_) | LocalAttachmentEvent::SyncRequired(_)
    )
}

#[cfg(unix)]
fn terminal_event_from_local(
    event: LocalAttachmentEvent,
    remote_alias: Option<&str>,
) -> Result<Option<TerminalViewEvent>, DaemonError> {
    match event {
        LocalAttachmentEvent::Snapshot(snapshot) => terminal_snapshot_from_wire(snapshot)
            .map(TerminalViewEvent::Snapshot)
            .map(Some),
        LocalAttachmentEvent::Delta(delta) => terminal_delta_from_wire(delta)
            .map(TerminalViewEvent::Delta)
            .map(Some),
        LocalAttachmentEvent::HistoryPage(page) => terminal_history_from_wire(page)
            .map(TerminalViewEvent::History)
            .map(Some),
        LocalAttachmentEvent::ViewportFrame(frame) => terminal_viewport_from_wire(frame)
            .map(TerminalViewEvent::Viewport)
            .map(Some),
        LocalAttachmentEvent::ConnectionStatus(status) => {
            let device = remote_alias.ok_or_else(|| {
                terminal_protocol_error("local terminal received remote connection status")
            })?;
            let path = match zterm_proto::v1::TerminalConnectionPath::try_from(status.path)
                .map_err(|_| terminal_protocol_error("unknown terminal connection path"))?
            {
                zterm_proto::v1::TerminalConnectionPath::Unknown => {
                    TerminalViewConnectionPath::Unknown
                }
                zterm_proto::v1::TerminalConnectionPath::Direct => {
                    TerminalViewConnectionPath::Direct
                }
                zterm_proto::v1::TerminalConnectionPath::Relay => TerminalViewConnectionPath::Relay,
                zterm_proto::v1::TerminalConnectionPath::Unspecified => {
                    return Err(terminal_protocol_error(
                        "terminal connection path was unspecified",
                    ));
                }
            };
            Ok(Some(TerminalViewEvent::ConnectionStatus(
                TerminalViewConnectionStatus {
                    device: device.to_owned(),
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
            let reason = match zterm_proto::v1::TerminalSessionEndReason::try_from(ended.reason)
                .map_err(|_| terminal_protocol_error("unknown terminal session end reason"))?
            {
                zterm_proto::v1::TerminalSessionEndReason::NaturalExit => {
                    TerminalViewEndReason::NaturalExit
                }
                zterm_proto::v1::TerminalSessionEndReason::ExplicitClose => {
                    TerminalViewEndReason::ExplicitClose
                }
                zterm_proto::v1::TerminalSessionEndReason::DaemonStop => {
                    TerminalViewEndReason::DaemonStop
                }
                zterm_proto::v1::TerminalSessionEndReason::DriverFailure => {
                    TerminalViewEndReason::DriverFailure
                }
                zterm_proto::v1::TerminalSessionEndReason::Unspecified => {
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
        LocalAttachmentEvent::TransportState(_) | LocalAttachmentEvent::Takeover(_) => Ok(None),
    }
}

#[cfg(unix)]
fn terminal_snapshot_from_wire(
    snapshot: zterm_proto::v1::TerminalSnapshot,
) -> Result<TerminalViewSnapshot, DaemonError> {
    Ok(TerminalViewSnapshot {
        revision: Revision::new(snapshot.revision),
        size: terminal_size_from_wire(snapshot.rows, snapshot.columns)?,
        active_screen: terminal_active_screen_from_wire(snapshot.active_screen)?,
        modes: terminal_modes_from_wire(snapshot.modes)?,
        recent_history_ansi: snapshot.recent_history_ansi,
        screen_ansi: snapshot.screen_ansi,
        scroll_metrics: terminal_scroll_metrics_from_wire(snapshot.scroll_metrics)?,
    })
}

#[cfg(unix)]
fn terminal_delta_from_wire(
    delta: zterm_proto::v1::TerminalDelta,
) -> Result<TerminalViewDelta, DaemonError> {
    Ok(TerminalViewDelta {
        from_revision: Revision::new(delta.from_revision),
        to_revision: Revision::new(delta.to_revision),
        size: terminal_size_from_wire(delta.rows, delta.columns)?,
        active_screen: terminal_active_screen_from_wire(delta.active_screen)?,
        modes: terminal_modes_from_wire(delta.modes)?,
        ansi: delta.ansi,
        scroll_metrics: terminal_scroll_metrics_from_wire(delta.scroll_metrics)?,
    })
}

#[cfg(unix)]
fn terminal_history_from_wire(
    page: zterm_proto::v1::TerminalHistoryPage,
) -> Result<TerminalHistoryResult, DaemonError> {
    let outcome = zterm_proto::v1::TerminalHistoryOutcome::try_from(page.outcome)
        .map_err(|_| terminal_protocol_error("unknown terminal history outcome"))?;
    match outcome {
        zterm_proto::v1::TerminalHistoryOutcome::Ok => {
            let cursor = page
                .cursor
                .ok_or_else(|| terminal_protocol_error("terminal history page omitted cursor"))?;
            Ok(TerminalHistoryResult::Page(TerminalHistoryPage {
                cursor: TerminalHistoryCursor {
                    epoch: Revision::new(cursor.epoch),
                    revision: Revision::new(cursor.revision),
                    start_row: cursor.start_row,
                    row_count: cursor.row_count,
                    oldest_row: cursor.oldest_row,
                    newest_row: cursor.newest_row,
                },
                rows: page.rows,
            }))
        }
        zterm_proto::v1::TerminalHistoryOutcome::Changed => {
            Ok(TerminalHistoryResult::HistoryChanged {
                epoch: Revision::new(page.current_epoch),
                revision: Revision::new(page.current_revision),
            })
        }
        zterm_proto::v1::TerminalHistoryOutcome::Gap => Ok(TerminalHistoryResult::HistoryGap {
            epoch: Revision::new(page.current_epoch),
            revision: Revision::new(page.current_revision),
        }),
        zterm_proto::v1::TerminalHistoryOutcome::Unspecified => Err(terminal_protocol_error(
            "terminal history outcome was unspecified",
        )),
    }
}

#[cfg(unix)]
fn terminal_scroll_metrics_from_wire(
    metrics: Option<zterm_proto::v1::TerminalScrollMetrics>,
) -> Result<Option<TerminalScrollMetrics>, DaemonError> {
    metrics
        .map(|metrics| {
            let viewport_rows = u16::try_from(metrics.viewport_rows).map_err(|_| {
                terminal_protocol_error("terminal viewport rows are outside the supported range")
            })?;
            let metrics = TerminalScrollMetrics {
                epoch: Revision::new(metrics.epoch),
                revision: Revision::new(metrics.revision),
                offset_from_bottom: metrics.offset_from_bottom,
                max_offset_from_bottom: metrics.max_offset_from_bottom,
                viewport_rows,
            };
            metrics
                .is_valid()
                .then_some(metrics)
                .ok_or_else(|| terminal_protocol_error("terminal scroll metrics are invalid"))
        })
        .transpose()
}

#[cfg(unix)]
fn terminal_viewport_from_wire(
    frame: zterm_proto::v1::TerminalViewportFrame,
) -> Result<TerminalViewportResult, DaemonError> {
    let outcome = zterm_proto::v1::TerminalViewportOutcome::try_from(frame.outcome)
        .map_err(|_| terminal_protocol_error("unknown terminal viewport outcome"))?;
    match outcome {
        zterm_proto::v1::TerminalViewportOutcome::Frame => {
            let metrics = terminal_scroll_metrics_from_wire(frame.metrics)?.ok_or_else(|| {
                terminal_protocol_error("terminal viewport frame omitted metrics")
            })?;
            let disposition =
                match zterm_proto::v1::TerminalViewportDisposition::try_from(frame.disposition)
                    .map_err(|_| terminal_protocol_error("unknown terminal viewport disposition"))?
                {
                    zterm_proto::v1::TerminalViewportDisposition::Exact => {
                        TerminalViewportDisposition::Exact
                    }
                    zterm_proto::v1::TerminalViewportDisposition::Rebased => {
                        TerminalViewportDisposition::Rebased
                    }
                    zterm_proto::v1::TerminalViewportDisposition::Unspecified => {
                        return Err(terminal_protocol_error(
                            "terminal viewport disposition was unspecified",
                        ));
                    }
                };
            Ok(TerminalViewportResult::Frame(TerminalViewportFrame {
                disposition,
                metrics,
                rows: frame.rows,
            }))
        }
        zterm_proto::v1::TerminalViewportOutcome::Live => {
            let metrics = terminal_scroll_metrics_from_wire(frame.metrics)?
                .ok_or_else(|| terminal_protocol_error("terminal live viewport omitted metrics"))?;
            Ok(TerminalViewportResult::Live(metrics))
        }
        zterm_proto::v1::TerminalViewportOutcome::Changed => {
            Ok(TerminalViewportResult::HistoryChanged {
                epoch: Revision::new(frame.current_epoch),
                revision: Revision::new(frame.current_revision),
            })
        }
        zterm_proto::v1::TerminalViewportOutcome::Gap => Ok(TerminalViewportResult::HistoryGap {
            epoch: Revision::new(frame.current_epoch),
            revision: Revision::new(frame.current_revision),
        }),
        zterm_proto::v1::TerminalViewportOutcome::Unspecified => Err(terminal_protocol_error(
            "terminal viewport outcome was unspecified",
        )),
    }
}

#[cfg(unix)]
fn terminal_size_from_wire(rows: u32, columns: u32) -> Result<TerminalSize, DaemonError> {
    let rows = u16::try_from(rows)
        .ok()
        .filter(|rows| *rows > 0)
        .ok_or_else(|| terminal_protocol_error("terminal rows are outside the supported range"))?;
    let columns = u16::try_from(columns)
        .ok()
        .filter(|columns| *columns > 0)
        .ok_or_else(|| {
            terminal_protocol_error("terminal columns are outside the supported range")
        })?;
    Ok(TerminalSize::new(rows, columns))
}

#[cfg(unix)]
fn terminal_active_screen_from_wire(value: i32) -> Result<ActiveScreen, DaemonError> {
    match zterm_proto::v1::TerminalActiveScreen::try_from(value)
        .map_err(|_| terminal_protocol_error("unknown terminal active screen"))?
    {
        zterm_proto::v1::TerminalActiveScreen::Main => Ok(ActiveScreen::Main),
        zterm_proto::v1::TerminalActiveScreen::Alternate => Ok(ActiveScreen::Alternate),
        zterm_proto::v1::TerminalActiveScreen::Unspecified => Err(terminal_protocol_error(
            "terminal active screen was unspecified",
        )),
    }
}

#[cfg(unix)]
fn terminal_modes_from_wire(
    modes: Option<zterm_proto::v1::TerminalModes>,
) -> Result<TerminalModes, DaemonError> {
    let modes = modes.ok_or_else(|| terminal_protocol_error("terminal update omitted modes"))?;
    let mouse_mode = match modes.mouse_mode {
        0 => TerminalMouseMode::None,
        1 => TerminalMouseMode::Press,
        2 => TerminalMouseMode::PressRelease,
        3 => TerminalMouseMode::ButtonMotion,
        4 => TerminalMouseMode::AnyMotion,
        _ => return Err(terminal_protocol_error("unknown terminal mouse mode")),
    };
    let mouse_encoding = match modes.mouse_encoding {
        0 => TerminalMouseEncoding::Default,
        1 => TerminalMouseEncoding::Utf8,
        2 => TerminalMouseEncoding::Sgr,
        _ => return Err(terminal_protocol_error("unknown terminal mouse encoding")),
    };
    Ok(TerminalModes {
        application_keypad: modes.application_keypad,
        application_cursor: modes.application_cursor,
        bracketed_paste: modes.bracketed_paste,
        focus_reporting: modes.focus_reporting,
        alternate_scroll: modes.alternate_scroll,
        mouse_mode,
        mouse_encoding,
    })
}

#[cfg(unix)]
fn terminal_transport_state_from_wire(
    value: i32,
) -> Result<TerminalViewTransportState, DaemonError> {
    match zterm_proto::v1::TerminalTransportState::try_from(value)
        .map_err(|_| terminal_protocol_error("unknown terminal transport state"))?
    {
        zterm_proto::v1::TerminalTransportState::Preparing => {
            Ok(TerminalViewTransportState::Preparing)
        }
        zterm_proto::v1::TerminalTransportState::Synchronizing => {
            Ok(TerminalViewTransportState::Synchronizing)
        }
        zterm_proto::v1::TerminalTransportState::Active => Ok(TerminalViewTransportState::Active),
        zterm_proto::v1::TerminalTransportState::Reconnecting => {
            Ok(TerminalViewTransportState::Reconnecting)
        }
        zterm_proto::v1::TerminalTransportState::Unspecified => Err(terminal_protocol_error(
            "terminal transport state was unspecified",
        )),
    }
}

#[cfg(unix)]
fn terminal_protocol_error(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

/// Side-effect-free impact projection for an identity reset confirmation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityResetPreflight {
    /// Whether any validated managed state root currently exists.
    pub state_present: bool,
    /// Whether committed managed state currently exists.
    pub configured: bool,
    /// Public identity which will be destroyed, when configured.
    pub device_id: Option<DeviceId>,
    /// Canonical public endpoint text which will be destroyed.
    pub endpoint_id: Option<String>,
    /// Whether the daemon answered the preflight status request.
    pub daemon_running: bool,
    /// Active Sessions which would be ended by a forced reset.
    pub active_session_names: Vec<String>,
}

impl IdentityResetPreflight {
    /// Current number of active Sessions affected by reset.
    #[must_use]
    pub fn active_session_count(&self) -> usize {
        self.active_session_names.len()
    }
}

/// Result of a completed or retry-safe already-complete identity reset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityResetResult {
    /// Whether a managed state root was removed by this invocation.
    pub removed: bool,
    /// Public identity destroyed by this invocation, when any.
    pub previous_device_id: Option<DeviceId>,
}

/// Result of one explicit authenticated binary update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateResult {
    /// Version replaced by this invocation.
    pub previous_version: String,
    /// Authenticated version now installed.
    pub installed_version: String,
    /// Sessions ended only after candidate verification and explicit force.
    pub ended_session_names: Vec<String>,
}

/// Side-effect-free uninstall impact bound to the current executable and identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UninstallPreflight {
    /// Existing identity/state impact.
    pub identity: IdentityResetPreflight,
    /// Current binary version that will be removed.
    pub version: String,
    /// Current exact build target.
    pub target: String,
}

/// Result of a completed or retry-safe uninstall.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UninstallResult {
    /// Whether managed identity state was removed by this invocation.
    pub state_removed: bool,
    /// Whether the validated running executable was unlinked.
    pub executable_removed: bool,
    /// Public identity destroyed by this invocation, when any.
    pub previous_device_id: Option<DeviceId>,
}

#[cfg(unix)]
fn require_update_daemon_compatible(
    version: &str,
    wire_major: u32,
    state_schema: u32,
) -> Result<(), DaemonError> {
    let build = zterm_core::BuildIdentity::current();
    if version != build.version
        || wire_major != build.wire_major
        || state_schema != build.state_schema
    {
        return Err(DaemonError::new(
            DomainErrorKind::UpdateRejected,
            "running daemon build is incompatible with this CLI; stop it with the matching binary before updating",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn require_update_session_force(impact: &SessionImpact, force: bool) -> Result<(), DaemonError> {
    if (impact.interruption_required || impact.active_session_count > 0) && !force {
        return Err(DaemonError::new(
            DomainErrorKind::UpdateRejected,
            format!(
                "{} active session(s) would be interrupted; retry with --force",
                impact.active_session_count
            ),
        ));
    }
    Ok(())
}

fn require_identity_reset_session_force(
    preflight: &IdentityResetPreflight,
    force: bool,
) -> Result<(), DaemonError> {
    if preflight.active_session_count() > 0 && !force {
        return Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            format!(
                "{} active session(s) would be interrupted; retry with --force",
                preflight.active_session_count()
            ),
        ));
    }
    Ok(())
}

/// Daemon-owned paths and launcher used by one CLI invocation.
#[derive(Clone)]
pub struct LocalRuntime {
    paths: UserPaths,
    launcher: DaemonLauncher,
}

impl fmt::Debug for LocalRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalRuntime")
            .field("uid", &self.paths.uid())
            .finish_non_exhaustive()
    }
}

impl LocalRuntime {
    /// Resolves the effective user's product paths and current executable.
    pub fn current() -> Result<Self, DaemonError> {
        Ok(Self {
            paths: crate::lifecycle::production_user_paths()?,
            launcher: DaemonLauncher::current()?,
        })
    }

    /// Creates a task-private runtime with an explicit launcher.
    #[doc(hidden)]
    #[must_use]
    pub const fn for_test(paths: UserPaths, launcher: DaemonLauncher) -> Self {
        Self { paths, launcher }
    }

    /// Validates or creates setup and explicitly ensures one daemon.
    pub async fn setup(&self, requested: &ValidatedConfig) -> Result<BootstrapResult, DaemonError> {
        setup_and_ensure(&self.paths, requested, &self.launcher).await
    }

    /// Authenticates, preflights, and atomically installs one explicit update.
    pub async fn update(
        &self,
        exact_tag: Option<&str>,
        force: bool,
    ) -> Result<UpdateResult, DaemonError> {
        #[cfg(unix)]
        {
            let executable = self.launcher.executable();
            crate::distribution::validate_managed_executable(executable, self.paths.uid())?;
            let selection = crate::distribution::ReleaseSelection::parse(exact_tag)?;
            let prepared = crate::distribution::prepare_update(selection).await?;

            let client = LocalClient::new(self.paths.socket());
            let (daemon_running, impact) = match client.status().await {
                Ok(status) => {
                    require_update_daemon_compatible(
                        &status.version,
                        status.protocol.wire_major,
                        status.protocol.state_schema,
                    )?;
                    (true, client.update_preflight().await?)
                }
                Err(error) if error.kind() == DomainErrorKind::DaemonStopped => (
                    false,
                    SessionImpact {
                        active_session_count: 0,
                        active_session_names: Vec::new(),
                        stopping: false,
                        interruption_required: false,
                    },
                ),
                Err(error) => return Err(error),
            };
            require_update_session_force(&impact, force)?;
            if daemon_running {
                client.stop(force).await?;
                wait_until_stopped(&self.paths).await?;
            }

            let state_present = managed_root_exists(&self.paths)?;
            let lifecycle = if state_present {
                let lock = acquire_lifecycle_lock(&self.paths, Instant::now()).await?;
                if probe_readiness(&self.paths).await?.is_some() {
                    return Err(DaemonError::new(
                        DomainErrorKind::UpdateRejected,
                        "daemon restarted while update was waiting for lifecycle ownership",
                    ));
                }
                ensure_daemon_ownership_released(&self.paths)?;
                Some(lock)
            } else {
                None
            };

            let mut source = fs::File::open(prepared.candidate()).map_err(|_| {
                DaemonError::new(
                    DomainErrorKind::ReleaseArtifactInvalid,
                    "verified update candidate became unavailable before activation",
                )
            })?;
            let activation = zterm_platform::user_state::activate_executable(
                executable,
                self.paths.uid(),
                |output| std::io::copy(&mut source, output).map(|_| ()),
            )
            .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;

            let post_activation =
                crate::distribution::verify_activated_candidate(executable, prepared.manifest())
                    .and_then(|()| {
                        if state_present {
                            crate::distribution::write_install_metadata(
                                &self.paths,
                                executable,
                                Some(prepared.manifest()),
                            )?;
                        }
                        Ok(())
                    });
            if let Err(error) = post_activation {
                activation.rollback().map_err(|rollback| {
                    DaemonError::new(
                        DomainErrorKind::PathUnsafe,
                        format!(
                            "update activation failed and rollback could not complete: {rollback}"
                        ),
                    )
                })?;
                return Err(error);
            }
            activation.commit().map_err(|error| {
                DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string())
            })?;
            drop(lifecycle);

            Ok(UpdateResult {
                previous_version: zterm_core::BuildIdentity::current().version.to_owned(),
                installed_version: prepared.version().to_owned(),
                ended_session_names: impact.active_session_names,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (exact_tag, force);
            Err(unsupported_command_platform())
        }
    }

    /// Observes uninstall impact without stopping, spawning, or deleting anything.
    pub async fn uninstall_preflight(&self) -> Result<UninstallPreflight, DaemonError> {
        crate::distribution::validate_managed_executable(
            self.launcher.executable(),
            self.paths.uid(),
        )?;
        let build = zterm_core::BuildIdentity::current();
        Ok(UninstallPreflight {
            identity: self.identity_reset_preflight().await?,
            version: build.version.to_owned(),
            target: build.target.to_owned(),
        })
    }

    /// Removes validated managed state, then unlinks only the running executable.
    pub async fn uninstall(
        &self,
        expected_device_id: Option<DeviceId>,
        force: bool,
    ) -> Result<UninstallResult, DaemonError> {
        #[cfg(unix)]
        {
            let executable = self.launcher.executable();
            crate::distribution::validate_managed_executable(executable, self.paths.uid())?;
            let reset = self.reset_identity(expected_device_id, force).await?;
            zterm_platform::user_state::remove_owned_executable(executable, self.paths.uid())
                .map_err(|error| {
                    DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string())
                })?;
            Ok(UninstallResult {
                state_removed: reset.removed,
                executable_removed: true,
                previous_device_id: reset.previous_device_id,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (expected_device_id, force);
            Err(unsupported_command_platform())
        }
    }

    /// Observes current setup/daemon state without spawning or creating files.
    pub async fn observe(&self) -> Result<ObservedState, DaemonError> {
        match LocalClient::new(self.paths.socket()).status().await {
            Ok(status) => Ok(ObservedState::Running(status)),
            Err(error) if error.kind() == DomainErrorKind::DaemonStopped => {
                match crate::bootstrap::validate_committed_setup(&self.paths) {
                    Ok(setup) => Ok(ObservedState::ConfiguredStopped(setup)),
                    Err(error) if error.kind() == DomainErrorKind::NotSetup => {
                        Ok(ObservedState::NotConfigured)
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    /// Explicitly ensures one daemon after its caller validated setup.
    async fn ensure(&self) -> Result<DaemonReadiness, DaemonError> {
        self.launcher.ensure(&self.paths).await
    }

    /// Validates committed setup, then singleflights one daemon without
    /// exposing its socket or launcher to the public command layer.
    pub async fn ensure_configured_daemon(&self) -> Result<DaemonReadiness, DaemonError> {
        match self.observe().await? {
            ObservedState::NotConfigured => Err(not_setup_for_command()),
            ObservedState::Running(_) | ObservedState::ConfiguredStopped(_) => {
                self.launcher.ensure(&self.paths).await
            }
        }
    }

    /// Creates a bounded one-time pairing ticket through the configured daemon.
    pub async fn pair_create(&self, ttl_seconds: u32) -> Result<PairTicketText, DaemonError> {
        self.ensure_configured_daemon().await?;
        #[cfg(unix)]
        {
            LocalPairingClient::new(self.paths.socket())
                .create(ttl_seconds)
                .await
        }
        #[cfg(not(unix))]
        {
            let _ = ttl_seconds;
            Err(unsupported_command_platform())
        }
    }

    /// Accepts a zeroizing ticket in the outbound direction.
    pub async fn pair_accept(
        &self,
        ticket: PairTicketText,
        alias: Option<&str>,
    ) -> Result<CommandDeviceSummary, DaemonError> {
        let alias = alias
            .map(|value| {
                DeviceAlias::new(value.to_owned()).map_err(|error| {
                    DaemonError::new(DomainErrorKind::InvalidDeviceAlias, error.to_string())
                })
            })
            .transpose()?;
        self.ensure_configured_daemon().await?;
        #[cfg(unix)]
        {
            LocalPairingClient::new(self.paths.socket())
                .accept(ticket, alias.as_ref())
                .await
                .map(Into::into)
        }
        #[cfg(not(unix))]
        {
            drop((ticket, alias));
            Err(unsupported_command_platform())
        }
    }

    /// Lists route-free directional device summaries.
    pub async fn device_list(&self) -> Result<Vec<CommandDeviceSummary>, DaemonError> {
        self.ensure_configured_daemon().await?;
        LocalDeviceClient::new(self.paths.socket())
            .list()
            .await
            .map(|devices| devices.into_iter().map(Into::into).collect())
    }

    /// Resolves one exact management selector without mutating either direction.
    pub async fn device_resolve(
        &self,
        selector: &str,
    ) -> Result<CommandDeviceSummary, DaemonError> {
        self.ensure_configured_daemon().await?;
        let devices = LocalDeviceClient::new(self.paths.socket()).list().await?;
        resolve_management_device(selector, &devices)
            .cloned()
            .map(Into::into)
    }

    /// Renames only an outbound known-device alias.
    pub async fn device_rename(
        &self,
        selector: &str,
        new_alias: &str,
    ) -> Result<CommandDeviceSummary, DaemonError> {
        let alias = DeviceAlias::new(new_alias.to_owned()).map_err(|error| {
            DaemonError::new(DomainErrorKind::InvalidDeviceAlias, error.to_string())
        })?;
        self.ensure_configured_daemon().await?;
        let client = LocalDeviceClient::new(self.paths.socket());
        let devices = client.list().await?;
        let selected = resolve_management_device(selector, &devices)?;
        if !selected.outbound_known() {
            return Err(DaemonError::new(
                DomainErrorKind::OutboundDirectionDenied,
                "device rename requires an outbound known-device record",
            ));
        }
        client
            .rename(selected.device_id(), &alias)
            .await
            .map(Into::into)
    }

    /// Revokes only inbound authorization for one confirmed exact device.
    pub async fn device_revoke(
        &self,
        device_id: DeviceId,
    ) -> Result<CommandDeviceSummary, DaemonError> {
        self.ensure_configured_daemon().await?;
        let client = LocalDeviceClient::new(self.paths.socket());
        let devices = client.list().await?;
        let selected = devices
            .iter()
            .find(|summary| summary.device_id() == device_id)
            .ok_or_else(|| {
                DaemonError::new(
                    DomainErrorKind::DeviceNotFound,
                    "no device has the confirmed exact identity",
                )
            })?;
        if selected.auth_status() == AuthorizationStatus::None {
            return Err(DaemonError::new(
                DomainErrorKind::Unauthorized,
                "device revoke requires an inbound authorization record",
            ));
        }
        client.revoke(device_id).await.map(Into::into)
    }

    /// Lists Sessions on one exact local or outbound remote target.
    pub async fn session_list(
        &self,
        target: &str,
    ) -> Result<Vec<CommandSessionSummary>, DaemonError> {
        let (client, target) = self.configured_session_target(target).await?;
        client
            .list_sessions_at(target)
            .await
            .map(|sessions| sessions.into_iter().map(Into::into).collect())
    }

    /// Resolves close impact once and freezes the exact target through confirmation.
    pub async fn session_close_preflight(
        &self,
        target: &str,
        selector: &str,
    ) -> Result<SessionClosePreflight, DaemonError> {
        let (client, target) = self.configured_session_target(target).await?;
        let session_id = resolve_session_id(&client, target, selector).await?;
        let summary = client
            .list_sessions_at(target)
            .await?
            .into_iter()
            .find(|summary| summary.session_id == session_id)
            .map(CommandSessionSummary::from)
            .ok_or_else(|| {
                DaemonError::new(
                    DomainErrorKind::SessionNotFound,
                    "no live session has the exact requested identity",
                )
            })?;
        Ok(SessionClosePreflight { target, summary })
    }

    /// Creates one Session while retaining the exact resolved target for attach.
    pub async fn session_create_for_attach(
        &self,
        target: &str,
        name: &str,
        working_directory: Option<&Path>,
        viewport: Option<TerminalSize>,
    ) -> Result<CreatedSession, DaemonError> {
        let name = parse_session_name(name)?;
        let (client, target) = self.configured_session_target(target).await?;
        let summary = client
            .create_session_at(target, &name, working_directory, viewport)
            .await?
            .into();
        Ok(CreatedSession { target, summary })
    }

    /// Attaches the exact Session and target returned by create without alias re-resolution.
    pub async fn attach_created(
        &self,
        created: &CreatedSession,
        viewport: Option<TerminalSize>,
    ) -> Result<PreparedTerminalView, DaemonError> {
        self.ensure_configured_daemon().await?;
        self.attach_resolved(
            created.target,
            Some(SessionSelector::Id(created.summary.session_id)),
            false,
            false,
            viewport,
        )
        .await
    }

    /// Renames one Session selected by exact full ID or exact current name.
    pub async fn session_rename(
        &self,
        target: &str,
        selector: &str,
        new_name: &str,
    ) -> Result<CommandSessionSummary, DaemonError> {
        let new_name = parse_session_name(new_name)?;
        let (client, target) = self.configured_session_target(target).await?;
        let session_id = resolve_session_id(&client, target, selector).await?;
        client
            .rename_session_at(target, session_id, &new_name)
            .await
            .map(Into::into)
    }

    /// Commits a confirmed close against its original frozen target and Session ID.
    pub async fn session_close_confirmed(
        &self,
        preflight: SessionClosePreflight,
    ) -> Result<CommandSessionSummary, DaemonError> {
        self.ensure_configured_daemon().await?;
        LocalClient::new(self.paths.socket())
            .close_session_at(preflight.target, preflight.summary.session_id)
            .await
            .map(Into::into)
    }

    /// Prepares an opaque local socket view for the later terminal UI.
    pub async fn attach(
        &self,
        target: &str,
        selector: Option<&str>,
        create_main: bool,
        takeover: bool,
        viewport: Option<TerminalSize>,
    ) -> Result<PreparedTerminalView, DaemonError> {
        let (_, target) = self.configured_session_target(target).await?;
        let selector = selector.map(parse_session_selector).transpose()?;
        self.attach_resolved(target, selector, create_main, takeover, viewport)
            .await
    }

    /// Observes reset impact without spawning, stopping, or creating files.
    pub async fn identity_reset_preflight(&self) -> Result<IdentityResetPreflight, DaemonError> {
        match self.observe().await? {
            ObservedState::Running(status) => Ok(IdentityResetPreflight {
                state_present: true,
                configured: true,
                device_id: Some(status.device_id),
                endpoint_id: Some(status.endpoint_id),
                daemon_running: true,
                active_session_names: status.active_session_names,
            }),
            ObservedState::ConfiguredStopped(setup) => Ok(IdentityResetPreflight {
                state_present: true,
                configured: true,
                device_id: Some(setup.device_id),
                endpoint_id: Some(setup.endpoint_id),
                daemon_running: false,
                active_session_names: Vec::new(),
            }),
            ObservedState::NotConfigured => {
                let state_present = managed_root_exists(&self.paths)?;
                let identity = if state_present {
                    partial_public_identity(&self.paths)?
                } else {
                    None
                };
                Ok(IdentityResetPreflight {
                    state_present,
                    configured: false,
                    device_id: identity.as_ref().map(|identity| identity.device_id()),
                    endpoint_id: identity.map(|identity| identity.endpoint_id()),
                    daemon_running: false,
                    active_session_names: Vec::new(),
                })
            }
        }
    }

    /// Stops the daemon and removes only its fully validated managed state.
    ///
    /// Callers must first render and confirm [`IdentityResetPreflight`]. The
    /// expected identity binds that confirmation to the destructive commit.
    pub async fn reset_identity(
        &self,
        expected_device_id: Option<DeviceId>,
        force: bool,
    ) -> Result<IdentityResetResult, DaemonError> {
        self.reset_identity_with_stop_timeout(
            expected_device_id,
            force,
            IDENTITY_RESET_STOP_TIMEOUT,
        )
        .await
    }

    async fn reset_identity_with_stop_timeout(
        &self,
        expected_device_id: Option<DeviceId>,
        force: bool,
        stop_timeout: Duration,
    ) -> Result<IdentityResetResult, DaemonError> {
        let preflight = self.identity_reset_preflight().await?;
        if !preflight.state_present {
            return Ok(IdentityResetResult {
                removed: false,
                previous_device_id: None,
            });
        }
        let current_device_id = preflight.device_id;
        if expected_device_id != current_device_id {
            return Err(DaemonError::new(
                DomainErrorKind::IdentityStateMismatch,
                "the configured identity changed after reset confirmation",
            ));
        }
        require_identity_reset_session_force(&preflight, force)?;

        let stop_deadline = Instant::now() + stop_timeout;
        tokio::time::timeout_at(
            tokio::time::Instant::from_std(stop_deadline),
            self.stop(force),
        )
        .await
        .map_err(|_| identity_reset_stop_timeout())??;

        #[cfg(unix)]
        {
            wait_until_identity_reset_stop(&self.paths, stop_deadline).await?;
            if !managed_root_exists(&self.paths)? {
                return Ok(IdentityResetResult {
                    removed: false,
                    previous_device_id: current_device_id,
                });
            }
            let lifecycle = acquire_lifecycle_lock(&self.paths, Instant::now()).await?;
            if probe_readiness(&self.paths).await?.is_some() {
                return Err(DaemonError::new(
                    DomainErrorKind::DaemonStopped,
                    "daemon restarted while identity reset was waiting for lifecycle ownership",
                ));
            }
            ensure_daemon_ownership_released(&self.paths)?;
            let locked_identity = partial_public_identity(&self.paths)?;
            if locked_identity
                .as_ref()
                .map(crate::identity::DeviceIdentity::device_id)
                != current_device_id
            {
                return Err(DaemonError::new(
                    DomainErrorKind::IdentityStateMismatch,
                    "the committed identity changed after reset confirmation",
                ));
            }
            if preflight.configured {
                let committed = crate::bootstrap::validate_committed_setup(&self.paths)?;
                if Some(committed.device_id) != current_device_id {
                    return Err(DaemonError::new(
                        DomainErrorKind::IdentityStateMismatch,
                        "the committed identity changed after reset confirmation",
                    ));
                }
            }
            zterm_platform::user_state::remove_managed_state_root(&self.paths, &lifecycle)
                .map_err(|error| {
                    DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string())
                })?;
            drop(lifecycle);
            Ok(IdentityResetResult {
                removed: true,
                previous_device_id: current_device_id,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = current_device_id;
            Err(unsupported_command_platform())
        }
    }

    async fn configured_session_target(
        &self,
        selector: &str,
    ) -> Result<(LocalClient, ResolvedSessionTarget), DaemonError> {
        self.ensure_configured_daemon().await?;
        let client = LocalClient::new(self.paths.socket());
        let target = client.resolve_session_target(selector).await?;
        Ok((client, target))
    }

    async fn attach_resolved(
        &self,
        target: ResolvedSessionTarget,
        selector: Option<SessionSelector>,
        create_main: bool,
        takeover: bool,
        viewport: Option<TerminalSize>,
    ) -> Result<PreparedTerminalView, DaemonError> {
        #[cfg(unix)]
        {
            let remote_alias = if let Some(device_id) = target.device_id() {
                let devices = LocalDeviceClient::new(self.paths.socket().to_path_buf())
                    .list()
                    .await?;
                let alias = devices
                    .into_iter()
                    .find(|device| device.device_id() == device_id)
                    .and_then(|device| device.alias().cloned())
                    .ok_or_else(|| {
                        DaemonError::new(
                            DomainErrorKind::DeviceNotFound,
                            "the exact remote target no longer has a local alias",
                        )
                    })?;
                Some(alias.as_str().to_owned())
            } else {
                None
            };
            let client = LocalAttachmentClient::connect_resolved(
                self.paths.socket(),
                target,
                selector,
                create_main,
                takeover,
                viewport,
            )
            .await?;
            let initial_snapshot = terminal_snapshot_from_wire(client.initial_snapshot().clone())?;
            Ok(PreparedTerminalView {
                session_id: client.session_id(),
                attachment_id: client.attachment_id(),
                initial_snapshot,
                takeover,
                remote_alias,
                client,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (target, selector, create_main, takeover, viewport);
            Err(unsupported_command_platform())
        }
    }

    /// Stops the daemon if running; already-stopped is a successful no-op.
    pub async fn stop(&self, force: bool) -> Result<Option<SessionImpact>, DaemonError> {
        let client = LocalClient::new(self.paths.socket());
        match client.status().await {
            Ok(status) => {
                if status.active_session_count > 0 && !force {
                    return Err(DaemonError::new(
                        DomainErrorKind::Cancelled,
                        format!(
                            "{} active session(s) would be interrupted; retry with --force",
                            status.active_session_count
                        ),
                    ));
                }
                client.stop(force).await.map(Some)
            }
            Err(error) if error.kind() == DomainErrorKind::DaemonStopped => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Stops when needed, waits for shutdown, then explicitly ensures one daemon.
    pub async fn restart(&self, force: bool) -> Result<DaemonReadiness, DaemonError> {
        match self.observe().await? {
            ObservedState::NotConfigured => return Err(not_setup_for_command()),
            ObservedState::ConfiguredStopped(_) => {}
            ObservedState::Running(_) => {
                if self.stop(force).await?.is_some() {
                    wait_until_stopped(&self.paths).await?;
                }
            }
        }
        self.ensure().await
    }

    /// Returns a bounded recent log tail without starting a daemon.
    pub fn log_tail(&self, requested_lines: usize) -> Result<Vec<String>, DaemonError> {
        let lines = requested_lines.min(MAX_LOG_LINES);
        let metadata = match fs::symlink_metadata(self.paths.daemon_log()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(DaemonError::new(
                    DomainErrorKind::PathUnsafe,
                    error.to_string(),
                ));
            }
        };
        zterm_platform::user_state::validate_regular_file(
            self.paths.daemon_log(),
            self.paths.uid(),
        )
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
        let mut file = fs::File::open(self.paths.daemon_log())
            .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
        let start = metadata.len().saturating_sub(MAX_LOG_BYTES);
        file.seek(SeekFrom::Start(start))
            .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
        let mut bytes =
            Vec::with_capacity(usize::try_from(metadata.len().saturating_sub(start)).unwrap_or(0));
        file.read_to_end(&mut bytes)
            .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(text
            .lines()
            .rev()
            .take(lines)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(str::to_owned)
            .collect())
    }

    /// Runs local-only diagnostics without spawning or using the network.
    pub async fn doctor(&self) -> DoctorReport {
        let mut checks = Vec::new();
        let observed = self.observe().await;
        let network = inspect_network_observation(&observed);
        let setup_complete = matches!(
            observed,
            Ok(ObservedState::Running(_) | ObservedState::ConfiguredStopped(_))
        );
        let daemon_running = matches!(observed, Ok(ObservedState::Running(_)));
        let state = match observed {
            Ok(ObservedState::Running(_)) => DoctorCheck {
                name: "setup",
                ok: true,
                detail: "setup complete; daemon is running".to_owned(),
            },
            Ok(ObservedState::ConfiguredStopped(_)) => DoctorCheck {
                name: "setup",
                ok: true,
                detail: "setup complete; daemon is stopped".to_owned(),
            },
            Ok(ObservedState::NotConfigured) => DoctorCheck {
                name: "setup",
                ok: false,
                detail: "zterm is not configured".to_owned(),
            },
            Err(error) => DoctorCheck {
                name: "setup",
                ok: false,
                detail: error.to_string(),
            },
        };
        checks.push(state);
        checks.push(DoctorCheck {
            name: "autostart",
            ok: true,
            detail: lifecycle_limitation().to_owned(),
        });
        checks.push(network);
        checks.push(inspect_account_home(&self.paths));
        checks.push(inspect_login_shell(&self.paths));
        checks.push(inspect_state_paths(&self.paths, setup_complete));
        checks.push(inspect_local_ipc(&self.paths, daemon_running));
        DoctorReport {
            healthy: checks.iter().all(|check| check.ok),
            checks,
        }
    }
}

fn not_setup_for_command() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::NotSetup,
        "zterm is not configured; run `zterm setup`",
    )
}

#[cfg(not(unix))]
fn unsupported_command_platform() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::UnsupportedPlatform,
        "public daemon commands are Unix-only in the current milestone",
    )
}

fn parse_session_name(value: &str) -> Result<SessionName, DaemonError> {
    SessionName::new(value.to_owned())
        .map_err(|error| DaemonError::new(DomainErrorKind::InvalidSessionName, error.to_string()))
}

fn parse_session_selector(value: &str) -> Result<SessionSelector, DaemonError> {
    let bytes = value.as_bytes();
    let canonical_id = bytes.len() == SessionId::CANONICAL_TEXT_LENGTH
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte));
    if canonical_id {
        return value
            .parse::<SessionId>()
            .map(SessionSelector::Id)
            .map_err(|error| {
                DaemonError::new(
                    DomainErrorKind::SessionNotFound,
                    format!("invalid canonical session ID: {error}"),
                )
            });
    }
    parse_session_name(value).map(SessionSelector::Name)
}

async fn resolve_session_id(
    client: &LocalClient,
    target: ResolvedSessionTarget,
    selector: &str,
) -> Result<SessionId, DaemonError> {
    match parse_session_selector(selector)? {
        SessionSelector::Id(session_id) => Ok(session_id),
        SessionSelector::Name(name) => client
            .list_sessions_at(target)
            .await?
            .into_iter()
            .find(|summary| summary.name == name)
            .map(|summary| summary.session_id)
            .ok_or_else(|| {
                DaemonError::new(
                    DomainErrorKind::SessionNotFound,
                    "no live session has the exact requested name",
                )
            }),
    }
}

fn resolve_management_device<'a>(
    selector: &str,
    devices: &'a [DeviceSummary],
) -> Result<&'a DeviceSummary, DaemonError> {
    let alias_match = devices.iter().find(|summary| {
        summary
            .alias()
            .is_some_and(|alias| alias.as_str() == selector)
    });
    let bytes = selector.as_bytes();
    let looks_hex = !bytes.is_empty() && bytes.iter().all(u8::is_ascii_hexdigit);

    if bytes.len() == DeviceId::CANONICAL_TEXT_LENGTH && looks_hex {
        if bytes.iter().any(u8::is_ascii_uppercase) {
            return Err(DaemonError::new(
                DomainErrorKind::InvalidTargetSelector,
                "device IDs must use the canonical lowercase hexadecimal form",
            ));
        }
        let device_id = selector.parse::<DeviceId>().map_err(|error| {
            DaemonError::new(
                DomainErrorKind::InvalidTargetSelector,
                format!("invalid canonical device ID: {error}"),
            )
        })?;
        if alias_match.is_some_and(|summary| summary.device_id() != device_id) {
            return Err(DaemonError::new(
                DomainErrorKind::InvalidTargetSelector,
                "selector is ambiguous between an exact alias and a canonical device ID",
            ));
        }
        return devices
            .iter()
            .find(|summary| summary.device_id() == device_id)
            .ok_or_else(|| {
                DaemonError::new(
                    DomainErrorKind::DeviceNotFound,
                    "no device has the exact requested identity",
                )
            });
    }

    if let Some(summary) = alias_match {
        return Ok(summary);
    }
    DeviceAlias::new(selector.to_owned()).map_err(|error| {
        DaemonError::new(
            DomainErrorKind::InvalidTargetSelector,
            format!("invalid exact device alias: {error}"),
        )
    })?;
    if looks_hex {
        return Err(DaemonError::new(
            DomainErrorKind::InvalidTargetSelector,
            "short and prefix device IDs are not accepted",
        ));
    }
    Err(DaemonError::new(
        DomainErrorKind::DeviceNotFound,
        "no device has the exact requested alias",
    ))
}

fn managed_root_exists(paths: &UserPaths) -> Result<bool, DaemonError> {
    match fs::symlink_metadata(paths.state_root()) {
        Ok(_) => {
            zterm_platform::user_state::validate_directory(paths.state_root(), paths.uid())
                .map_err(|error| {
                    DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string())
                })?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(DaemonError::new(
            DomainErrorKind::PathUnsafe,
            error.to_string(),
        )),
    }
}

fn partial_public_identity(
    paths: &UserPaths,
) -> Result<Option<crate::identity::DeviceIdentity>, DaemonError> {
    match fs::symlink_metadata(paths.identity()) {
        Ok(_) => crate::identity::DeviceIdentity::load(paths).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DaemonError::new(
            DomainErrorKind::PathUnsafe,
            error.to_string(),
        )),
    }
}

#[cfg(unix)]
fn ensure_daemon_ownership_released(paths: &UserPaths) -> Result<(), DaemonError> {
    use zterm_platform::user_state::{ExistingLockState, inspect_existing_lock};

    match inspect_existing_lock(paths.daemon_lock(), paths.uid())
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?
    {
        ExistingLockState::Missing | ExistingLockState::Unlocked => {}
        ExistingLockState::Locked => {
            return Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                "daemon ownership is still active after the bounded stop",
            ));
        }
    }
    if zterm_platform::local_unix::inspect_daemon_socket(paths)
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?
    {
        return Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            "daemon socket is still present after the bounded stop",
        ));
    }
    Ok(())
}

#[cfg(unix)]
async fn wait_until_identity_reset_stop(
    paths: &UserPaths,
    deadline: Instant,
) -> Result<(), DaemonError> {
    loop {
        let readiness_released = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            probe_readiness(paths),
        )
        .await
        {
            Ok(Ok(None)) => true,
            Ok(Ok(Some(_))) => false,
            Ok(Err(error)) if error.kind() == DomainErrorKind::DaemonStartTimeout => false,
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(identity_reset_stop_timeout()),
        };
        let ownership_released = match ensure_daemon_ownership_released(paths) {
            Ok(()) => true,
            Err(error) if error.kind() == DomainErrorKind::Cancelled => false,
            Err(error) => return Err(error),
        };
        if readiness_released && ownership_released {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(identity_reset_stop_timeout());
        }
        tokio::time::sleep(Duration::from_millis(20).min(remaining)).await;
    }
}

fn identity_reset_stop_timeout() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DeadlineExceeded,
        "identity reset could not prove readiness, socket, and daemon-lock release before its stop deadline",
    )
}

fn inspect_network_observation(observed: &Result<ObservedState, DaemonError>) -> DoctorCheck {
    match observed {
        Ok(ObservedState::Running(status)) => {
            let network = &status.network;
            let diagnostic = network
                .diagnostic
                .map_or("none", crate::network::NetworkDiagnostic::code);
            let ok = !matches!(
                network.state,
                crate::network::NetworkState::Degraded
                    | crate::network::NetworkState::Stopping
                    | crate::network::NetworkState::Stopped
            ) && network.diagnostic.is_none();
            DoctorCheck {
                name: "network",
                ok,
                detail: format!(
                    "state={}, endpoint_bound={}, bind_attempts={}, publish={}, lookup={}, authenticated={}, primary={}, streams={}, direct_paths={}, relay_paths={}, diagnostic={diagnostic}",
                    network.state.as_str(),
                    network.endpoint_bound,
                    network.bind_attempts,
                    network.publish.as_str(),
                    network.lookup.as_str(),
                    network.authenticated_connection_count,
                    network.primary_connection_count,
                    network.active_stream_count,
                    network.direct_path_count,
                    network.relay_path_count,
                ),
            }
        }
        Ok(ObservedState::ConfiguredStopped(_)) => DoctorCheck {
            name: "network",
            ok: true,
            detail: "daemon is stopped; network observation was not attempted".to_owned(),
        },
        Ok(ObservedState::NotConfigured) => DoctorCheck {
            name: "network",
            ok: true,
            detail: "setup is incomplete; network observation was not attempted".to_owned(),
        },
        Err(error) => DoctorCheck {
            name: "network",
            ok: false,
            detail: format!("network observation unavailable: {error}"),
        },
    }
}

fn inspect_account_home(paths: &UserPaths) -> DoctorCheck {
    let result = fs::metadata(paths.home()).and_then(|metadata| {
        if metadata.is_dir() {
            Ok(metadata)
        } else {
            Err(std::io::Error::other("account home is not a directory"))
        }
    });
    match result {
        Ok(metadata) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.uid() != paths.uid() {
                    return DoctorCheck {
                        name: "account_home",
                        ok: false,
                        detail: format!(
                            "{} is owned by UID {}, expected {}",
                            paths.home().display(),
                            metadata.uid(),
                            paths.uid()
                        ),
                    };
                }
            }
            #[cfg(not(unix))]
            let _ = metadata;
            DoctorCheck {
                name: "account_home",
                ok: true,
                detail: paths.home().display().to_string(),
            }
        }
        Err(error) => DoctorCheck {
            name: "account_home",
            ok: false,
            detail: format!("{}: {error}", paths.home().display()),
        },
    }
}

fn inspect_login_shell(paths: &UserPaths) -> DoctorCheck {
    let shell = paths.login_shell();
    let metadata = fs::metadata(shell);
    let valid = metadata.as_ref().is_ok_and(|metadata| metadata.is_file()) && shell.is_absolute();
    #[cfg(unix)]
    let valid = {
        use std::os::unix::fs::PermissionsExt;
        valid
            && metadata
                .as_ref()
                .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    };
    DoctorCheck {
        name: "login_shell",
        ok: valid,
        detail: if valid {
            shell.display().to_string()
        } else {
            format!(
                "{} is not an absolute executable regular file",
                shell.display()
            )
        },
    }
}

fn inspect_state_paths(paths: &UserPaths, setup_complete: bool) -> DoctorCheck {
    let mut failures = Vec::new();
    inspect_directory(paths.state_root(), paths.uid(), &mut failures);
    inspect_directory(paths.logs(), paths.uid(), &mut failures);
    for path in [paths.identity(), paths.config(), paths.database()] {
        inspect_required_file(path, paths.uid(), &mut failures);
    }
    for path in [
        paths.install_metadata(),
        paths.lifecycle_lock(),
        paths.daemon_lock(),
        paths.daemon_log(),
    ] {
        inspect_optional_file(path, paths.uid(), &mut failures);
    }
    if !setup_complete && failures.is_empty() {
        failures.push("committed setup is incomplete".to_owned());
    }
    DoctorCheck {
        name: "state_paths",
        ok: failures.is_empty(),
        detail: if failures.is_empty() {
            format!("{} is consistent", paths.state_root().display())
        } else {
            failures.join("; ")
        },
    }
}

fn inspect_directory(path: &Path, uid: u32, failures: &mut Vec<String>) {
    if let Err(error) = zterm_platform::user_state::validate_directory(path, uid) {
        failures.push(error.to_string());
    }
}

fn inspect_required_file(path: &Path, uid: u32, failures: &mut Vec<String>) {
    match fs::symlink_metadata(path) {
        Ok(_) => inspect_optional_file(path, uid, failures),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            failures.push(format!(
                "required managed file is missing: {}",
                path.display()
            ));
        }
        Err(error) => failures.push(format!("{}: {error}", path.display())),
    }
}

fn inspect_optional_file(path: &Path, uid: u32, failures: &mut Vec<String>) {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            if let Err(error) = zterm_platform::user_state::validate_regular_file(path, uid) {
                failures.push(error.to_string());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => failures.push(format!("{}: {error}", path.display())),
    }
}

fn inspect_local_ipc(paths: &UserPaths, daemon_running: bool) -> DoctorCheck {
    #[cfg(unix)]
    {
        use zterm_platform::user_state::{ExistingLockState, inspect_existing_lock};

        let runtime = match fs::symlink_metadata(paths.runtime_dir()) {
            Ok(_) => {
                zterm_platform::user_state::validate_directory(paths.runtime_dir(), paths.uid())
                    .map_err(|error| error.to_string())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !daemon_running => Ok(()),
            Err(error) => Err(format!("{}: {error}", paths.runtime_dir().display())),
        };
        let socket = zterm_platform::local_unix::inspect_daemon_socket(paths)
            .map_err(|error| error.to_string());
        let lock = inspect_existing_lock(paths.daemon_lock(), paths.uid())
            .map_err(|error| error.to_string());

        let state_matches = matches!(
            (daemon_running, &socket, &lock),
            (true, Ok(true), Ok(ExistingLockState::Locked))
                | (
                    false,
                    Ok(false),
                    Ok(ExistingLockState::Missing | ExistingLockState::Unlocked)
                )
        );
        let ok = runtime.is_ok() && state_matches;
        let runtime_detail = runtime
            .as_ref()
            .map_or_else(|error| error.as_str(), |()| "ok");
        DoctorCheck {
            name: "local_ipc",
            ok,
            detail: if ok {
                if daemon_running {
                    "owned socket and daemon lock are active".to_owned()
                } else {
                    "daemon is stopped with no stale socket or held lock".to_owned()
                }
            } else {
                format!(
                    "runtime={}, socket={socket:?}, daemon_lock={lock:?}, expected_running={daemon_running}",
                    runtime_detail
                )
            },
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (paths, daemon_running);
        DoctorCheck {
            name: "local_ipc",
            ok: false,
            detail: "Unix local daemon integration is unsupported on this platform".to_owned(),
        }
    }
}

const fn lifecycle_limitation() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "1.0 has no boot/login autostart; systemd-logind may end the daemon after logout unless the host keeps user processes"
    }
    #[cfg(not(target_os = "linux"))]
    {
        "1.0 has no boot/login autostart; setup/restart starts the daemon on demand"
    }
}

/// Validates or creates setup and explicitly ensures one daemon.
///
/// A double readiness probe under `lifecycle.lock` prevents offline SQLite
/// bootstrap from racing a concurrently launched daemon's `StoreActor`.
pub async fn setup_and_ensure(
    paths: &UserPaths,
    requested: &ValidatedConfig,
    launcher: &DaemonLauncher,
) -> Result<BootstrapResult, DaemonError> {
    #[cfg(unix)]
    {
        paths
            .prepare_state_directories()
            .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
        if probe_readiness(paths).await?.is_some() {
            return validate_running_setup(paths, requested).await;
        }

        let lifecycle = acquire_lifecycle_lock(paths, Instant::now()).await?;
        if probe_readiness(paths).await?.is_some() {
            drop(lifecycle);
            return validate_running_setup(paths, requested).await;
        }
        let result = bootstrap_with_lock_held(paths, requested)?;
        crate::distribution::write_install_metadata(paths, launcher.executable(), None)?;
        drop(lifecycle);
        launcher.ensure(paths).await?;
        Ok(result)
    }
    #[cfg(not(unix))]
    {
        let _ = (paths, requested, launcher);
        Err(DaemonError::new(
            DomainErrorKind::UnsupportedPlatform,
            "local daemon setup is Unix-only in the current milestone",
        ))
    }
}

#[cfg(unix)]
async fn validate_running_setup(
    paths: &UserPaths,
    requested: &ValidatedConfig,
) -> Result<BootstrapResult, DaemonError> {
    let validated = LocalClient::new(paths.socket())
        .validate_setup(requested)
        .await?;
    Ok(BootstrapResult {
        device_id: validated.device_id,
        endpoint_id: validated.endpoint_id,
        config: requested.clone(),
    })
}

async fn wait_until_stopped(paths: &UserPaths) -> Result<(), DaemonError> {
    let started = Instant::now();
    loop {
        #[cfg(unix)]
        let ownership_released = match ensure_daemon_ownership_released(paths) {
            Ok(()) => true,
            Err(error) if error.kind() == DomainErrorKind::Cancelled => false,
            Err(error) => return Err(error),
        };
        #[cfg(not(unix))]
        let ownership_released = true;
        // Do not open a new readiness connection while the retiring daemon
        // still owns its lock or socket: listener shutdown may accept and
        // close that probe without a response. Once ownership is absent, the
        // normal readiness owner can prove the stopped observation.
        let readiness_released = if ownership_released {
            probe_readiness(paths).await?.is_none()
        } else {
            false
        };
        if readiness_released && ownership_released {
            return Ok(());
        }
        if started.elapsed() >= std::time::Duration::from_secs(5) {
            return Err(DaemonError::new(
                DomainErrorKind::DaemonStartTimeout,
                "daemon readiness, socket, and ownership were not released within 5 seconds",
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::collections::VecDeque;

    #[cfg(unix)]
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use zterm_core::{Capabilities, DeviceId};
    #[cfg(unix)]
    use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v1};

    use super::*;
    use crate::network::{
        AddressServiceState, NetworkDiagnostic, NetworkObservation, NetworkState,
    };
    use crate::service::ProtocolStatus;

    #[test]
    fn distribution_lifecycle_requires_force_for_active_sessions() {
        #[cfg(unix)]
        {
            let update_impact = SessionImpact {
                active_session_count: 1,
                active_session_names: vec!["main".to_owned()],
                stopping: false,
                interruption_required: true,
            };
            assert_eq!(
                require_update_session_force(&update_impact, false)
                    .expect_err("update must refuse active Sessions without force")
                    .kind(),
                DomainErrorKind::UpdateRejected
            );
            require_update_session_force(&update_impact, true)
                .expect("forced update may cross the already-rendered impact boundary");
        }

        let uninstall_impact = IdentityResetPreflight {
            state_present: true,
            configured: true,
            device_id: Some(DeviceId::from_array([0x71; DeviceId::LENGTH])),
            endpoint_id: Some("public-endpoint".to_owned()),
            daemon_running: true,
            active_session_names: vec!["main".to_owned()],
        };
        assert_eq!(
            require_identity_reset_session_force(&uninstall_impact, false)
                .expect_err("uninstall must refuse active Sessions without force")
                .kind(),
            DomainErrorKind::Cancelled
        );
        require_identity_reset_session_force(&uninstall_impact, true)
            .expect("forced uninstall may cross the already-rendered impact boundary");
    }

    #[cfg(unix)]
    #[test]
    fn update_rejects_every_incompatible_running_daemon_identity_field() {
        let build = zterm_core::BuildIdentity::current();
        require_update_daemon_compatible(build.version, build.wire_major, build.state_schema)
            .expect("matching daemon identity");

        for (version, wire_major, state_schema) in [
            ("0.0.0", build.wire_major, build.state_schema),
            (build.version, build.wire_major + 1, build.state_schema),
            (build.version, build.wire_major, build.state_schema + 1),
        ] {
            assert_eq!(
                require_update_daemon_compatible(version, wire_major, state_schema)
                    .expect_err("incompatible daemon must be refused before stop")
                    .kind(),
                DomainErrorKind::UpdateRejected
            );
        }
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
    fn connection_status_projection_keeps_the_frozen_alias_and_redacts_private_identity() {
        const ALIAS_SENTINEL: &str = "远程-Mac-STATUS_ALIAS_SENTINEL";
        const ATTACHMENT_SENTINEL: &[u8; AttachmentId::LENGTH] = b"STATUS_ID_SENTIN";
        let attachment_id = AttachmentId::from_array(*ATTACHMENT_SENTINEL);
        let project = |path, rtt_ms| {
            terminal_event_from_local(
                LocalAttachmentEvent::ConnectionStatus(v1::TerminalConnectionStatusEvent {
                    attachment_id: Some(attachment_id.into()),
                    path,
                    rtt_ms,
                }),
                Some(ALIAS_SENTINEL),
            )
            .expect("same-UID status projects")
            .expect("same-UID status remains visible")
        };

        for (event, expected_path, expected_rtt) in [
            (
                project(v1::TerminalConnectionPath::Unknown as i32, None),
                TerminalViewConnectionPath::Unknown,
                None,
            ),
            (
                project(v1::TerminalConnectionPath::Direct as i32, Some(7)),
                TerminalViewConnectionPath::Direct,
                Some(7),
            ),
            (
                project(v1::TerminalConnectionPath::Relay as i32, Some(19)),
                TerminalViewConnectionPath::Relay,
                Some(19),
            ),
        ] {
            let TerminalViewEvent::ConnectionStatus(status) = event else {
                panic!("connection status projects to its typed view event");
            };
            assert_eq!(status.device(), ALIAS_SENTINEL);
            assert_eq!(status.path(), expected_path);
            assert_eq!(status.rtt_ms(), expected_rtt);
            let rendered = format!("{status:?}");
            assert!(!rendered.contains(ALIAS_SENTINEL));
            assert!(!rendered.contains(
                std::str::from_utf8(ATTACHMENT_SENTINEL).expect("ASCII attachment sentinel")
            ));
        }

        let error = terminal_event_from_local(
            LocalAttachmentEvent::ConnectionStatus(v1::TerminalConnectionStatusEvent {
                attachment_id: Some(attachment_id.into()),
                path: v1::TerminalConnectionPath::Direct as i32,
                rtt_ms: Some(7),
            }),
            None,
        )
        .expect_err("local-only views cannot accept remote connection status");
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
    }

    #[test]
    fn doctor_check_debug_redacts_detail_without_changing_public_diagnostics() {
        const DETAIL_SENTINEL: &str = "/private/tmp/DOCTOR_DETAIL_SENTINEL_69ba/state.sqlite3";
        let check = DoctorCheck {
            name: "state_paths",
            ok: false,
            detail: DETAIL_SENTINEL.to_owned(),
        };
        let rendered = format!("{check:?}");

        assert!(!rendered.contains(DETAIL_SENTINEL));
        assert!(rendered.contains("detail: \"[REDACTED]\""));
        assert!(rendered.contains(&format!("detail_len: {}", DETAIL_SENTINEL.len())));
        assert_eq!(check.detail, DETAIL_SENTINEL);
        assert_eq!(check, check.clone());
    }

    #[test]
    fn doctor_projects_only_redacted_typed_network_observation() {
        let device_id = DeviceId::from_array([0x52; 32]);
        let observed = Ok(ObservedState::Running(DaemonStatus {
            protocol: ProtocolStatus {
                wire_major: 1,
                state_schema: 1,
                capabilities: Capabilities::LOCAL_LIFECYCLE,
            },
            version: "test".to_owned(),
            phase: "test".to_owned(),
            device_id,
            endpoint_id: "public-endpoint".to_owned(),
            device_name: "doctor-host".to_owned(),
            infrastructure_profile: "official-n0".to_owned(),
            started_at_unix: 1,
            active_session_count: 0,
            active_session_names: Vec::new(),
            network: NetworkObservation {
                device_id,
                state: NetworkState::Degraded,
                endpoint_bound: true,
                bind_attempts: 3,
                home_relay: Some("https://relay.example.test".to_owned()),
                publish: AddressServiceState::Configured,
                lookup: AddressServiceState::Degraded,
                authenticated_connection_count: 4,
                primary_connection_count: 2,
                active_stream_count: 5,
                direct_path_count: 1,
                relay_path_count: 1,
                diagnostic: Some(NetworkDiagnostic::HomeRelayUnavailable),
            },
        }));

        let check = inspect_network_observation(&observed);
        assert_eq!(check.name, "network");
        assert!(!check.ok);
        assert!(check.detail.contains("state=degraded"));
        assert!(check.detail.contains("publish=configured"));
        assert!(check.detail.contains("direct_paths=1"));
        assert!(check.detail.contains("relay_paths=1"));
        assert!(check.detail.contains("home_relay_unavailable"));
        for forbidden in [
            "direct_ip",
            "route_cache",
            "pair_secret",
            "ticket",
            "relay.example.test",
        ] {
            assert!(!check.detail.contains(forbidden));
        }
    }

    #[test]
    fn doctor_skips_network_when_setup_is_absent() {
        let check = inspect_network_observation(&Ok(ObservedState::NotConfigured));
        assert!(check.ok);
        assert!(check.detail.contains("not attempted"));
    }

    #[test]
    fn management_resolution_is_exact_directional_and_ambiguity_safe() {
        let first_id = DeviceId::from_array([0x81; DeviceId::LENGTH]);
        let second_id = DeviceId::from_array([0x82; DeviceId::LENGTH]);
        let first = DeviceSummary::new(
            first_id,
            true,
            Some(DeviceAlias::new("laptop").expect("alias")),
            "Laptop",
            true,
            AuthorizationStatus::None,
            AuthGeneration::ZERO,
            0,
            1,
            false,
            0,
            0,
        )
        .expect("outbound summary");
        let second = DeviceSummary::new(
            second_id,
            false,
            None,
            "",
            false,
            AuthorizationStatus::Authorized,
            AuthGeneration::new(2).expect("generation"),
            1,
            2,
            true,
            1,
            1,
        )
        .expect("inbound summary");
        let devices = vec![first, second];

        assert_eq!(
            resolve_management_device("laptop", &devices)
                .expect("exact alias")
                .device_id(),
            first_id
        );
        assert_eq!(
            resolve_management_device(&second_id.to_string(), &devices)
                .expect("full inbound-only ID")
                .device_id(),
            second_id
        );
        assert_eq!(
            resolve_management_device("82828282", &devices)
                .expect_err("short IDs are rejected")
                .kind(),
            DomainErrorKind::InvalidTargetSelector
        );

        let ambiguous = DeviceSummary::new(
            first_id,
            true,
            Some(DeviceAlias::new(second_id.to_string()).expect("hex alias")),
            "Ambiguous Laptop",
            false,
            AuthorizationStatus::None,
            AuthGeneration::ZERO,
            0,
            0,
            false,
            0,
            0,
        )
        .expect("ambiguous outbound projection");
        assert_eq!(
            resolve_management_device(&second_id.to_string(), &[ambiguous, devices[1].clone()])
                .expect_err("ID/alias ambiguity is explicit")
                .kind(),
            DomainErrorKind::InvalidTargetSelector
        );
    }

    #[test]
    fn session_selector_uses_only_canonical_full_ids_or_exact_names() {
        let session_id = SessionId::from_array([0xab; SessionId::LENGTH]);
        assert_eq!(
            parse_session_selector(&session_id.to_string()).expect("canonical Session ID"),
            SessionSelector::Id(session_id)
        );
        let uppercase = session_id.to_string().to_uppercase();
        assert!(matches!(
            parse_session_selector(&uppercase).expect("uppercase text is an exact name"),
            SessionSelector::Name(name) if name.as_str() == uppercase
        ));
        assert!(matches!(
            parse_session_selector("build").expect("exact name"),
            SessionSelector::Name(name) if name.as_str() == "build"
        ));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn identity_reset_uses_one_deadline_for_socket_and_daemon_lock_release() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        use zterm_platform::user_state::FileLock;

        let temporary = tempfile::tempdir().expect("temporary root");
        let home = temporary.path().join("home");
        fs::create_dir(&home).expect("test home");
        let uid = fs::metadata(&home).expect("home metadata").uid();
        let paths = UserPaths::for_test(
            uid,
            home.clone(),
            home.join(".zterm"),
            temporary.path().join("runtime"),
        );
        paths.prepare_state_directories().expect("state dirs");
        let runtime = LocalRuntime::for_test(
            paths.clone(),
            DaemonLauncher::for_test("/does/not/exist".into(), "--must-not-run".to_owned()),
        );

        let daemon_lock = FileLock::try_acquire(paths.daemon_lock(), paths.uid())
            .expect("daemon lock probe")
            .expect("daemon lock");
        let locked_error = runtime
            .reset_identity_with_stop_timeout(None, true, Duration::from_millis(40))
            .await
            .expect_err("held daemon ownership blocks reset");
        assert_eq!(locked_error.kind(), DomainErrorKind::DeadlineExceeded);
        assert!(paths.state_root().exists());
        drop(daemon_lock);

        paths
            .prepare_runtime_directory()
            .expect("runtime directory");
        let stale = std::os::unix::net::UnixListener::bind(paths.socket())
            .expect("stale local socket fixture");
        fs::set_permissions(paths.socket(), fs::Permissions::from_mode(0o600))
            .expect("socket mode");
        drop(stale);
        let socket_error = runtime
            .reset_identity_with_stop_timeout(None, true, Duration::from_millis(40))
            .await
            .expect_err("owned socket path blocks reset");
        assert_eq!(socket_error.kind(), DomainErrorKind::DeadlineExceeded);
        assert!(paths.state_root().exists());
        fs::remove_file(paths.socket()).expect("remove stale socket fixture");

        let result = runtime
            .reset_identity_with_stop_timeout(None, true, Duration::from_secs(1))
            .await
            .expect("retry after complete ownership release");
        assert!(result.removed);
        assert!(!paths.state_root().exists());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn development_binary_cannot_update_or_uninstall_owned_executable_or_state() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let temporary = tempfile::tempdir().expect("temporary root");
        let home = temporary.path().join("home");
        let bin = home.join("bin");
        fs::create_dir_all(&bin).expect("owned install directory");
        let uid = fs::metadata(&home).expect("home metadata").uid();
        let paths = UserPaths::for_test(
            uid,
            home.clone(),
            home.join(".zterm"),
            temporary.path().join("runtime"),
        );
        let source = temporary.path().join("candidate");
        fs::write(&source, b"candidate executable").expect("candidate bytes");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).expect("candidate mode");
        let executable = bin.join("zterm");
        zterm_platform::user_state::install_executable(&source, &executable, uid)
            .expect("owned development executable");
        paths
            .prepare_state_directories()
            .expect("managed state fixture");

        let runtime = LocalRuntime::for_test(
            paths.clone(),
            DaemonLauncher::for_test(executable.clone(), "--must-not-run".to_owned()),
        );

        let update = runtime
            .update(None, false)
            .await
            .expect_err("development build must fail before release network access");
        assert_eq!(update.kind(), DomainErrorKind::PathUnsafe);
        let preflight = runtime
            .uninstall_preflight()
            .await
            .expect_err("development build must not offer destructive preflight");
        assert_eq!(preflight.kind(), DomainErrorKind::PathUnsafe);
        let uninstall = runtime
            .uninstall(None, false)
            .await
            .expect_err("development build must not remove binary or state");
        assert_eq!(uninstall.kind(), DomainErrorKind::PathUnsafe);
        assert!(executable.exists());
        assert!(paths.state_root().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_command_closure_prefers_typed_end_and_normalizes_plain_eof() {
        let session_id = SessionId::from_array([0xc1; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0xc2; AttachmentId::LENGTH]);
        let (mut client, mut peer) = LocalAttachmentClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        peer.write_all(
            &encode_message(
                WireKind::TerminalSessionEnded,
                0,
                0,
                &v1::TerminalSessionEnded {
                    session_id: Some(session_id.into()),
                    attachment_id: Some(attachment_id.into()),
                    reason: v1::TerminalSessionEndReason::NaturalExit as i32,
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
        let (response, received) = tokio::sync::oneshot::channel();
        correlate_terminal_command_closure(
            &mut client,
            &mut pending,
            None,
            &mut takeover_pending,
            &mut last_state,
            response,
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

        let (mut eof_client, eof_peer) = LocalAttachmentClient::terminal_driver_test_pair(
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
            None,
            &mut takeover_pending,
            &mut last_state,
            response,
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
    #[tokio::test]
    async fn legal_resize_after_terminal_event_was_queued_preserves_the_typed_outcome() {
        let session_id = SessionId::from_array([0xc5; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0xc6; AttachmentId::LENGTH]);
        let (client, mut peer) = LocalAttachmentClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        let view = spawn_terminal_driver(client, TerminalViewTransportState::Active, None, false);
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
                &v1::TerminalSessionEnded {
                    session_id: Some(session_id.into()),
                    attachment_id: Some(attachment_id.into()),
                    reason: v1::TerminalSessionEndReason::NaturalExit as i32,
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
        let (client, peer) = LocalAttachmentClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        let view = spawn_terminal_driver(client, TerminalViewTransportState::Active, None, false);
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
        remote_alias: Option<&str>,
    ) -> (
        VecDeque<Result<TerminalViewEvent, DaemonError>>,
        TerminalViewTransportState,
    ) {
        let (mut client, mut peer) = LocalAttachmentClient::terminal_driver_test_pair(
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
        let (response, received) = tokio::sync::oneshot::channel();
        assert!(
            !apply_terminal_driver_command(
                Some(TerminalDriverCommand::Resize {
                    size: TerminalSize::new(31, 97),
                    response,
                }),
                &mut client,
                &mut pending,
                remote_alias,
                remote_alias.is_some(),
                &mut takeover_pending,
                &mut last_state,
                &mut stop_after_pending,
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
                &v1::TerminalSessionEnded {
                    session_id: Some(session_id.into()),
                    attachment_id: Some(attachment_id.into()),
                    reason: v1::TerminalSessionEndReason::DaemonStop as i32,
                    exit_code: 0,
                    signal: String::new(),
                },
            )],
            None,
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
                &v1::TerminalLeaseLost {
                    attachment_id: Some(attachment_id.into()),
                    generation: 23,
                },
            )],
            None,
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
                    &v1::TerminalTransportStateEvent {
                        attachment_id: Some(attachment_id.into()),
                        state: v1::TerminalTransportState::Reconnecting as i32,
                    },
                ),
                closure_schedule_frame(
                    WireKind::TerminalConnectionStatusEvent,
                    &v1::TerminalConnectionStatusEvent {
                        attachment_id: Some(attachment_id.into()),
                        path: v1::TerminalConnectionPath::Unknown as i32,
                        rtt_ms: None,
                    },
                ),
                closure_schedule_frame(
                    WireKind::TerminalSyncRequired,
                    &v1::TerminalSyncRequired {
                        attachment_id: Some(attachment_id.into()),
                        latest_revision: 29,
                    },
                ),
            ],
            Some("frozen-peer"),
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
                &v1::ServiceError {
                    code: typed_error.kind().code().to_owned(),
                    message: typed_error.detail().to_owned(),
                },
            )],
            None,
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
            let initial: v1::TerminalSnapshotApplied = initial_ack
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
                        &v1::TerminalLeaseLost {
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
                        &v1::ServiceError {
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
        let (client, peer) = LocalAttachmentClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        let initial_snapshot =
            terminal_snapshot_from_wire(client.initial_snapshot().clone()).expect("test snapshot");
        (
            PreparedTerminalView {
                session_id,
                attachment_id,
                initial_snapshot,
                takeover,
                remote_alias: None,
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
    fn test_operation_lease() -> v1::SessionOperationLeaseResponse {
        v1::SessionOperationLeaseResponse {
            lease: Some(v1::OperationLease {
                daemon_incarnation: vec![0xb3; 16],
                ordinal: 1,
            }),
        }
    }

    #[cfg(unix)]
    fn test_takeover_response(session_id: SessionId) -> v1::SessionMutateResponse {
        v1::SessionMutateResponse {
            session: Some(v1::SessionSummary {
                session_id: Some(session_id.into()),
                name: "main".to_owned(),
                revision: 1,
                has_controller: true,
                working_directory: String::new(),
                viewport: Some(v1::TerminalViewport {
                    rows: 24,
                    columns: 80,
                }),
            }),
        }
    }
}
