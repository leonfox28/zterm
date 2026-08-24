//! Daemon-aware command backend shared by the thin CLI.

use std::fmt;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

use zterm_core::terminal::{ActiveScreen, TerminalSize};
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
use crate::local_ipc::{LocalAttachmentClient, LocalAttachmentEvent, LocalPairingClient};
use crate::local_ipc::{LocalClient, LocalDeviceClient};
use crate::pairing::PairTicketText;
use crate::service::{DaemonReadiness, DaemonStatus, SessionImpact};

const MAX_LOG_LINES: usize = 1_000;
const MAX_LOG_BYTES: u64 = 1024 * 1024;
const IDENTITY_RESET_STOP_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const TERMINAL_DRIVER_CAPACITY: usize = 8;

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
    recent_history_ansi: Vec<u8>,
    screen_ansi: Vec<u8>,
}

impl fmt::Debug for TerminalViewSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewSnapshot")
            .field("revision", &self.revision)
            .field("size", &self.size)
            .field("active_screen", &self.active_screen)
            .field("recent_history_ansi_len", &self.recent_history_ansi.len())
            .field("screen_ansi_len", &self.screen_ansi.len())
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
}

/// Daemon-authored merged update from one exact acknowledged revision.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalViewDelta {
    from_revision: Revision,
    to_revision: Revision,
    size: TerminalSize,
    active_screen: ActiveScreen,
    ansi: Vec<u8>,
}

impl fmt::Debug for TerminalViewDelta {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewDelta")
            .field("from_revision", &self.from_revision)
            .field("to_revision", &self.to_revision)
            .field("size", &self.size)
            .field("active_screen", &self.active_screen)
            .field("ansi_len", &self.ansi.len())
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

    /// Daemon-authored ANSI for this contiguous update.
    #[must_use]
    pub fn ansi(&self) -> &[u8] {
        &self.ansi
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
    /// Replace the local rendered state atomically.
    Snapshot(TerminalViewSnapshot),
    /// Apply one merged update only when its baseline is contiguous.
    Delta(TerminalViewDelta),
    /// Discard the current baseline and request a replacement snapshot.
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
            Self::Snapshot(snapshot) => formatter.debug_tuple("Snapshot").field(snapshot).finish(),
            Self::Delta(delta) => formatter.debug_tuple("Delta").field(delta).finish(),
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
    remote: bool,
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
            .field("remote", &self.remote)
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
        let initial_state = if self.remote || self.takeover {
            TerminalViewTransportState::Synchronizing
        } else {
            TerminalViewTransportState::Active
        };
        Ok(spawn_terminal_driver(
            self.client,
            initial_state,
            self.remote,
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
        self.submit(|response| TerminalDriverCommand::SnapshotApplied { revision, response })
            .await
    }

    /// Sends ordinary controller input. Callers must gate this on `Active`.
    pub async fn write_input(&self, bytes: Vec<u8>) -> Result<(), DaemonError> {
        self.submit(|response| TerminalDriverCommand::Input { bytes, response })
            .await
    }

    /// Sends the latest validated viewport. Callers coalesce while non-active.
    pub async fn resize(&self, size: TerminalSize) -> Result<(), DaemonError> {
        self.submit(|response| TerminalDriverCommand::Resize { size, response })
            .await
    }

    /// Requests a full replacement after a revision gap.
    pub async fn request_sync(&self, known_revision: Revision) -> Result<(), DaemonError> {
        self.submit(|response| TerminalDriverCommand::RequestSync {
            known_revision,
            response,
        })
        .await
    }

    /// Detaches this view while leaving the Session and PTY running.
    pub async fn detach(&self) -> Result<(), DaemonError> {
        self.submit(|response| TerminalDriverCommand::Detach { response })
            .await
    }

    #[cfg(unix)]
    async fn submit(
        &self,
        command: impl FnOnce(
            tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
        ) -> TerminalDriverCommand,
    ) -> Result<(), DaemonError> {
        let (response, receiver) = tokio::sync::oneshot::channel();
        self.sender
            .send(command(response))
            .await
            .map_err(|_| terminal_driver_closed())?;
        receiver.await.map_err(|_| terminal_driver_closed())?
    }

    #[cfg(not(unix))]
    async fn submit(
        &self,
        _command: impl FnOnce(
            tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
        ) -> TerminalDriverCommand,
    ) -> Result<(), DaemonError> {
        Err(unsupported_command_platform())
    }
}

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
    Detach {
        response: tokio::sync::oneshot::Sender<Result<(), DaemonError>>,
    },
}

#[cfg(unix)]
fn spawn_terminal_driver(
    client: LocalAttachmentClient,
    initial_state: TerminalViewTransportState,
    remote: bool,
    takeover: bool,
) -> TerminalViewIo {
    let (command_sender, command_receiver) = tokio::sync::mpsc::channel(TERMINAL_DRIVER_CAPACITY);
    let (event_sender, event_receiver) = tokio::sync::mpsc::channel(TERMINAL_DRIVER_CAPACITY);
    tokio::spawn(run_terminal_driver(
        client,
        command_receiver,
        event_sender,
        initial_state,
        remote,
        takeover,
    ));
    TerminalViewIo {
        reader: TerminalViewEventReader {
            receiver: event_receiver,
        },
        writer: TerminalViewCommandWriter {
            sender: command_sender,
        },
    }
}

#[cfg(unix)]
async fn run_terminal_driver(
    mut client: LocalAttachmentClient,
    mut commands: tokio::sync::mpsc::Receiver<TerminalDriverCommand>,
    events: tokio::sync::mpsc::Sender<Result<TerminalViewEvent, DaemonError>>,
    initial_state: TerminalViewTransportState,
    remote: bool,
    takeover: bool,
) {
    use std::collections::VecDeque;

    let mut pending = VecDeque::from([Ok(TerminalViewEvent::TransportState(initial_state))]);
    let mut stop_after_pending = false;
    let mut local_takeover_pending = takeover && !remote;
    let mut last_state = initial_state;

    loop {
        if pending.is_empty() {
            tokio::select! {
                command = commands.recv() => {
                    match handle_terminal_driver_command(command, &mut client).await {
                        TerminalDriverCommandResult::Continue => {}
                        TerminalDriverCommandResult::SnapshotApplied
                            if !remote
                                && !local_takeover_pending
                                && last_state != TerminalViewTransportState::Active =>
                        {
                            last_state = TerminalViewTransportState::Active;
                            pending.push_back(Ok(TerminalViewEvent::TransportState(last_state)));
                        }
                        TerminalDriverCommandResult::SnapshotApplied => {}
                        TerminalDriverCommandResult::Stop => return,
                    }
                }
                () = events.closed() => {
                    let _ = client.detach().await;
                    return;
                }
                event = client.read_next_event() => {
                    match event {
                        Ok(LocalAttachmentEvent::Takeover(_)) if local_takeover_pending => {
                            local_takeover_pending = false;
                            last_state = TerminalViewTransportState::Active;
                            pending.push_back(Ok(TerminalViewEvent::TransportState(last_state)));
                        }
                        Ok(LocalAttachmentEvent::Takeover(_)) => {}
                        Ok(LocalAttachmentEvent::TransportState(state)) => {
                            match terminal_transport_state_from_wire(state.state) {
                                Ok(TerminalViewTransportState::Preparing) => {}
                                Ok(state) if state == last_state => {}
                                Ok(state) => {
                                    last_state = state;
                                    pending.push_back(Ok(TerminalViewEvent::TransportState(state)));
                                }
                                Err(error) => {
                                    pending.push_back(Err(error));
                                    stop_after_pending = true;
                                }
                            }
                        }
                        Ok(event) => {
                            if local_event_requires_synchronizing(&event)
                                && last_state != TerminalViewTransportState::Synchronizing
                            {
                                last_state = TerminalViewTransportState::Synchronizing;
                                pending.push_back(Ok(TerminalViewEvent::TransportState(last_state)));
                            }
                            match terminal_event_from_local(event) {
                                Ok(Some(event)) => pending.push_back(Ok(event)),
                                Ok(None) => {}
                                Err(error) => {
                                    pending.push_back(Err(error));
                                    stop_after_pending = true;
                                }
                            }
                        }
                        Err(error) => {
                            pending.push_back(Err(error));
                            stop_after_pending = true;
                        }
                    }
                }
            }
            continue;
        }

        tokio::select! {
            command = commands.recv() => {
                match handle_terminal_driver_command(command, &mut client).await {
                    TerminalDriverCommandResult::Continue => {}
                    TerminalDriverCommandResult::SnapshotApplied
                        if !remote
                            && !local_takeover_pending
                            && last_state != TerminalViewTransportState::Active =>
                    {
                        last_state = TerminalViewTransportState::Active;
                        pending.push_back(Ok(TerminalViewEvent::TransportState(last_state)));
                    }
                    TerminalDriverCommandResult::SnapshotApplied => {}
                    TerminalDriverCommandResult::Stop => return,
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
                    return;
                }
            }
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
        TerminalDriverCommand::Detach { response } => (
            client.detach().await,
            response,
            TerminalDriverCommandResult::Stop,
        ),
    };
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
) -> Result<Option<TerminalViewEvent>, DaemonError> {
    match event {
        LocalAttachmentEvent::Snapshot(snapshot) => terminal_snapshot_from_wire(snapshot)
            .map(TerminalViewEvent::Snapshot)
            .map(Some),
        LocalAttachmentEvent::Delta(delta) => terminal_delta_from_wire(delta)
            .map(TerminalViewEvent::Delta)
            .map(Some),
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
        recent_history_ansi: snapshot.recent_history_ansi,
        screen_ansi: snapshot.screen_ansi,
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
        ansi: delta.ansi,
    })
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

fn terminal_protocol_error(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

fn terminal_driver_closed() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::Cancelled,
        "terminal attachment driver closed",
    )
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
        if preflight.active_session_count() > 0 && !force {
            return Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                format!(
                    "{} active session(s) would be interrupted; retry with --force",
                    preflight.active_session_count()
                ),
            ));
        }

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
                remote: client.is_remote(),
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
        if probe_readiness(paths).await?.is_none() {
            return Ok(());
        }
        if started.elapsed() >= std::time::Duration::from_secs(5) {
            return Err(DaemonError::new(
                DomainErrorKind::DaemonStartTimeout,
                "daemon did not stop within 5 seconds",
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

            let detach = read_terminal_test_frame(&mut daemon, &mut decoder, &mut queued).await;
            assert_eq!(detach.kind, WireKind::TerminalDetach);
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
        writer.detach().await.expect("detach local view");
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
                remote: false,
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
