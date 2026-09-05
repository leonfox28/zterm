//! One route-neutral attachment protocol and recovery owner.
use super::ipc::mutate_response;
use super::transport::{AttachmentTransport, AttachmentTransportItem};
use super::{
    DEFAULT_DEADLINE, decode_response, malformed, resolved_target_wire, resource_error,
    service_error,
};
use crate::{device_directory::ResolvedSessionTarget, error::DaemonError, service::protocol_error};
use ring::rand::{SecureRandom, SystemRandom};
use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use zterm_core::terminal::{
    TerminalClipboardWrite, TerminalHistoryWindowQuery, TerminalSurfaceDelta,
    TerminalSurfaceHistoryWindowResult, TerminalSurfaceSnapshot,
};
use zterm_core::{
    AttachmentId, DeviceId, DomainErrorKind, OperationId, OperationLease, ResumeViewId, Revision,
    SessionId, SessionSelector,
};
#[cfg(test)]
use zterm_proto::FrameDecoder;
use zterm_proto::{DecodedFrame, WireKind, encode_message, v2};

const MAX_DEFERRED_FRAMES: usize = 8;

/// One typed server message received on a local terminal attachment.
#[derive(Clone, PartialEq)]
#[doc(hidden)]
pub enum LocalAttachmentEvent {
    /// Latest frontend-owned attachment transport state.
    TransportState(v2::TerminalTransportStateEvent),
    /// Address-free selected path and RTT from the remote tunnel sideband.
    ConnectionStatus(v2::TerminalConnectionStatusEvent),
    /// A full host-authoritative replacement state.
    Snapshot(TerminalSurfaceSnapshot),
    /// A merged revision update from the acknowledged checkpoint.
    Delta(TerminalSurfaceDelta),
    /// One correlated exact semantic history-window outcome.
    HistoryWindow(TerminalSurfaceHistoryWindowResult),
    /// One validated latest-only child clipboard write.
    ClipboardWrite(TerminalClipboardWrite),
    /// The following snapshot must replace the client state atomically.
    SyncRequired(v2::TerminalSyncRequired),
    /// A prepared takeover committed successfully.
    Takeover(crate::session::SessionSummary),
    /// Another attachment replaced this controller.
    LeaseLost(v2::TerminalLeaseLost),
    /// The underlying session and PTY ended.
    SessionEnded(v2::TerminalSessionEnded),
}

impl fmt::Debug for LocalAttachmentEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TransportState(state) => formatter
                .debug_struct("TransportState")
                .field("state", &state.state)
                .finish_non_exhaustive(),
            Self::ConnectionStatus(status) => formatter
                .debug_struct("ConnectionStatus")
                .field("path", &status.path)
                .field("rtt_ms", &status.rtt_ms)
                .finish_non_exhaustive(),
            Self::Snapshot(snapshot) => formatter
                .debug_struct("SemanticSnapshot")
                .field("revision", &snapshot.revision)
                .field("row_count", &snapshot.surface.rows.len())
                .finish_non_exhaustive(),
            Self::Delta(delta) => formatter
                .debug_struct("SemanticDelta")
                .field("from_revision", &delta.from_revision)
                .field("to_revision", &delta.to_revision)
                .field("row_patch_count", &delta.row_patches.len())
                .finish_non_exhaustive(),
            Self::HistoryWindow(result) => formatter
                .debug_tuple("SemanticHistoryWindow")
                .field(result)
                .finish(),
            Self::ClipboardWrite(write) => formatter
                .debug_tuple("ClipboardWrite")
                .field(write)
                .finish(),
            Self::SyncRequired(required) => formatter
                .debug_struct("SyncRequired")
                .field("latest_revision", &required.latest_revision)
                .finish_non_exhaustive(),
            Self::Takeover(summary) => formatter.debug_tuple("Takeover").field(summary).finish(),
            Self::LeaseLost(lost) => formatter
                .debug_struct("LeaseLost")
                .field("generation", &lost.generation)
                .finish_non_exhaustive(),
            Self::SessionEnded(ended) => formatter
                .debug_struct("SessionEnded")
                .field("reason", &ended.reason)
                .field("exit_code", &ended.exit_code)
                .field("has_signal", &!ended.signal.is_empty())
                .finish_non_exhaustive(),
        }
    }
}

/// Opaque same-daemon retry token for one takeover whose response was lost.
///
/// It is intentionally process-memory only in M4. Callers must export and
/// retain it explicitly if they need fresh-process ambiguity recovery.
#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct LocalTakeoverRetryToken {
    operation_id: OperationId,
    session_id: SessionId,
}

impl fmt::Debug for LocalTakeoverRetryToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LocalTakeoverRetryToken([REDACTED])")
    }
}

/// Narrow lifecycle capability retained by a remote frontend view.
///
/// The production implementation delegates to the normal lifecycle-locked
/// daemon launcher. Local views never retain this capability: losing their
/// daemon also ends the local Session they were displaying.
pub(crate) trait RemoteDaemonRestarter: Send + Sync {
    fn ensure_running<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'a>>;
}

struct ReconnectedAttachment {
    transport: AttachmentTransport,
    attachment_id: AttachmentId,
    initial: LocalAttachmentEvent,
    paths: Vec<v2::LocalSessionTunnelPath>,
    terminal_size: zterm_core::terminal::TerminalSize,
}

/// Real same-UID duplex socket adapter for one frontend-owned terminal attachment.
#[doc(hidden)]
pub struct SessionClient {
    transport: AttachmentTransport,
    write_error: Option<DaemonError>,
    takeover_deadline: Option<tokio::time::Instant>,
    deferred: VecDeque<DecodedFrame>,
    pending_transport_events: VecDeque<LocalAttachmentEvent>,
    socket: PathBuf,
    session_id: SessionId,
    attachment_id: AttachmentId,
    target: ResolvedSessionTarget,
    resume_view_id: Option<ResumeViewId>,
    applied_revision: Arc<AtomicU64>,
    latest_viewport: zterm_core::terminal::TerminalSize,
    reconnect_pending: bool,
    force_full_sync: bool,
    initial_snapshot: Option<TerminalSurfaceSnapshot>,
    terminal_rows: u32,
    next_request_id: u64,
    operation_lease: Option<OperationLease>,
    next_operation_sequence: u64,
    pending_takeover_request_id: Option<u64>,
    pending_history_window: Option<(u64, TerminalHistoryWindowQuery)>,
    remote_daemon_restarter: Option<Arc<dyn RemoteDaemonRestarter>>,
}

impl fmt::Debug for SessionClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionClient")
            .field("session_id", &self.session_id)
            .field("attachment_id", &self.attachment_id)
            .field("has_initial_snapshot", &self.initial_snapshot.is_some())
            .field("queued_frames", &self.transport.queued_session_count())
            .field("deferred_frames", &self.deferred.len())
            .field("has_operation_lease", &self.operation_lease.is_some())
            .field(
                "has_pending_takeover",
                &self.pending_takeover_request_id.is_some(),
            )
            .field(
                "has_pending_history_window",
                &self.pending_history_window.is_some(),
            )
            .field(
                "has_remote_daemon_restarter",
                &self.remote_daemon_restarter.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl SessionClient {
    /// Opens one already-resolved local or remote view for the high-level
    /// command runtime without exposing the socket or target token to the CLI.
    pub(crate) async fn connect_resolved(
        socket: impl AsRef<Path>,
        target: ResolvedSessionTarget,
        selector: Option<SessionSelector>,
        create_main: bool,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            socket.as_ref(),
            target,
            selector,
            create_main,
            takeover,
            viewport,
        )
        .await
    }

    /// Attaches to the daemon-lifetime default `main` session, creating it if absent.
    pub async fn connect_main(
        socket: impl AsRef<Path>,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            socket.as_ref(),
            ResolvedSessionTarget::local(),
            None,
            true,
            false,
            viewport,
        )
        .await
    }

    /// Attaches to an existing session selected by stable ID or exact name.
    pub async fn connect_session(
        socket: impl AsRef<Path>,
        selector: SessionSelector,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            socket.as_ref(),
            ResolvedSessionTarget::local(),
            Some(selector),
            false,
            takeover,
            viewport,
        )
        .await
    }

    /// Opens one frontend-owned remote default view through an opaque daemon tunnel.
    #[doc(hidden)]
    pub async fn connect_remote_main(
        socket: impl AsRef<Path>,
        target: DeviceId,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            socket.as_ref(),
            ResolvedSessionTarget::device(target),
            None,
            true,
            false,
            viewport,
        )
        .await
    }

    /// Opens one frontend-owned remote named/ID view through an opaque daemon tunnel.
    #[doc(hidden)]
    pub async fn connect_remote_session(
        socket: impl AsRef<Path>,
        target: DeviceId,
        selector: SessionSelector,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            socket.as_ref(),
            ResolvedSessionTarget::device(target),
            Some(selector),
            false,
            takeover,
            viewport,
        )
        .await
    }

    async fn connect_inner(
        socket: &Path,
        target: ResolvedSessionTarget,
        selector: Option<SessionSelector>,
        create_main: bool,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        let deadline = super::control_deadline();
        let socket = socket.to_path_buf();
        let (session_id, session_name) = match selector {
            Some(SessionSelector::Id(session_id)) => (Some(session_id.into()), String::new()),
            Some(SessionSelector::Name(name)) => (None, name.to_string()),
            None => (None, String::new()),
        };
        let resume_view_id = if target.device_id().is_some() {
            Some(random_resume_view_id()?)
        } else {
            None
        };
        let request_id = 1;
        let bytes = encode_message(
            WireKind::TerminalAttachRequest,
            request_id,
            u32::try_from(DEFAULT_DEADLINE.as_millis()).unwrap_or(u32::MAX),
            &v2::TerminalAttachRequest {
                target: Some(resolved_target_wire(target)),
                session_id,
                takeover,
                session_name,
                create_main,
                viewport: viewport.map(Into::into),
                resume_view_id: resume_view_id.map(Into::into),
                known_revision: None,
            },
        )
        .map_err(protocol_error)?;
        let mut transport =
            tokio::time::timeout_at(deadline, AttachmentTransport::open(&socket, target))
                .await
                .map_err(|_| super::control_timeout())??;
        if let Err(error) = transport.write_until(&bytes, deadline).await {
            return Err(if create_main {
                create_main_outcome_unknown()
            } else {
                error
            });
        }

        // The outer result owns post-write ambiguity. The inner result is
        // reserved for a decoded, correlated ServiceError, which is already a
        // definitive result and must retain its exact domain category.
        let response = tokio::time::timeout_at(deadline, async {
            let mut pre_snapshot_states = Vec::new();
            let mut pre_snapshot_paths = Vec::new();
            let initial_snapshot = loop {
                let frame = match transport.read_item().await? {
                    AttachmentTransportItem::Session(frame) => frame,
                    AttachmentTransportItem::Path(path) => {
                        check_initial_capacity(
                            pre_snapshot_paths.len() + pre_snapshot_states.len(),
                        )?;
                        pre_snapshot_paths.push(path);
                        continue;
                    }
                };
                if frame.kind == WireKind::ServiceErrorResponse {
                    if frame.request_id != request_id {
                        return Err(malformed("initial terminal error correlation mismatch"));
                    }
                    return Ok(Err(service_error(&frame)?));
                }
                if frame.kind == WireKind::TerminalTransportStateEvent {
                    if target.device_id().is_some() {
                        return Err(malformed(
                            "remote target Session stream carried a same-UID-only transport state",
                        ));
                    }
                    let state: v2::TerminalTransportStateEvent = frame
                        .decode_message(WireKind::TerminalTransportStateEvent)
                        .map_err(protocol_error)?;
                    v2::TerminalTransportState::try_from(state.state)
                        .map_err(|_| malformed("unknown terminal transport state"))?;
                    check_initial_capacity(pre_snapshot_paths.len() + pre_snapshot_states.len())?;
                    pre_snapshot_states.push(state);
                    continue;
                }
                if frame.request_id != request_id
                    || frame.kind != WireKind::TerminalSemanticSnapshot
                {
                    return Err(malformed("initial terminal snapshot correlation mismatch"));
                }
                let snapshot: v2::TerminalSemanticSnapshot = frame
                    .decode_message(WireKind::TerminalSemanticSnapshot)
                    .map_err(protocol_error)?;
                break zterm_proto::terminal_surface_snapshot_from_message(snapshot)
                    .map_err(protocol_error)?;
            };
            let (session_id, attachment_id, initial_snapshot) = initial_snapshot;
            let terminal_rows = u32::from(initial_snapshot.surface.size.rows);
            for state in &pre_snapshot_states {
                let state_attachment: AttachmentId = state
                    .attachment_id
                    .clone()
                    .ok_or_else(|| malformed("transport state omitted attachment_id"))?
                    .try_into()
                    .map_err(protocol_error)?;
                if state_attachment != attachment_id {
                    return Err(malformed("transport state attachment_id mismatch"));
                }
            }
            let pending_transport_events =
                changed_tunnel_path_events_after_unknown(attachment_id, pre_snapshot_paths);
            Ok(Ok(Self {
                transport,
                write_error: None,
                takeover_deadline: None,
                deferred: VecDeque::new(),
                pending_transport_events,
                socket,
                session_id,
                attachment_id,
                target,
                resume_view_id,
                applied_revision: Arc::new(AtomicU64::new(initial_snapshot.revision.get())),
                latest_viewport: initial_snapshot.surface.size,
                reconnect_pending: false,
                force_full_sync: false,
                initial_snapshot: Some(initial_snapshot),
                terminal_rows,
                next_request_id: request_id + 1,
                operation_lease: None,
                next_operation_sequence: 1,
                pending_takeover_request_id: None,
                pending_history_window: None,
                remote_daemon_restarter: None,
            }))
        })
        .await;

        match response {
            Ok(Ok(result)) => result,
            Ok(Err(_)) if create_main => Err(create_main_outcome_unknown()),
            Ok(Err(error)) => Err(error),
            Err(_) if create_main => Err(create_main_outcome_unknown()),
            Err(_) => Err(DaemonError::new(
                DomainErrorKind::DeadlineExceeded,
                "timed out waiting for initial terminal snapshot",
            )),
        }
    }

    async fn reconnect_remote(&mut self) -> Result<(), DaemonError> {
        let resume_view_id = self.resume_view_id.ok_or_else(|| {
            malformed("only a remote terminal view may reconnect through a tunnel")
        })?;
        loop {
            match self.reconnect_remote_once(resume_view_id).await {
                Ok(reconnected) => {
                    self.transport = reconnected.transport;
                    self.write_error = None;
                    self.takeover_deadline = None;
                    self.attachment_id = reconnected.attachment_id;
                    self.terminal_rows = u32::from(reconnected.terminal_size.rows);
                    self.latest_viewport = reconnected.terminal_size;
                    self.deferred.clear();
                    self.pending_transport_events.clear();
                    self.operation_lease = None;
                    self.next_operation_sequence = 1;
                    self.pending_history_window = None;
                    self.pending_takeover_request_id = None;
                    self.reconnect_pending = false;
                    self.force_full_sync = false;
                    self.pending_transport_events
                        .push_back(LocalAttachmentEvent::TransportState(
                            terminal_transport_state(
                                self.attachment_id,
                                v2::TerminalTransportState::Synchronizing,
                            ),
                        ));
                    self.pending_transport_events.push_back(
                        LocalAttachmentEvent::ConnectionStatus(v2::TerminalConnectionStatusEvent {
                            attachment_id: Some(self.attachment_id.into()),
                            path: v2::TerminalConnectionPath::Unknown as i32,
                            rtt_ms: None,
                        }),
                    );
                    self.pending_transport_events
                        .extend(changed_tunnel_path_events_after_unknown(
                            self.attachment_id,
                            reconnected.paths,
                        ));
                    self.pending_transport_events.push_back(reconnected.initial);
                    return Ok(());
                }
                Err(error) if is_retryable_remote_reconnect(error.kind()) => {
                    if error.kind() == DomainErrorKind::DaemonStopped
                        && let Some(restarter) = self.remote_daemon_restarter.clone()
                    {
                        restarter.ensure_running().await?;
                        continue;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn reconnect_remote_once(
        &mut self,
        resume_view_id: ResumeViewId,
    ) -> Result<ReconnectedAttachment, DaemonError> {
        let deadline = super::control_deadline();
        let request_id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or_else(|| resource_error("terminal reconnect request ID exhausted"))?;
        let request = v2::TerminalAttachRequest {
            target: Some(resolved_target_wire(self.target)),
            session_id: Some(self.session_id.into()),
            takeover: false,
            session_name: String::new(),
            create_main: false,
            viewport: Some(self.latest_viewport.into()),
            resume_view_id: Some(resume_view_id.into()),
            known_revision: (!self.force_full_sync)
                .then(|| self.applied_revision.load(Ordering::Acquire)),
        };
        let bytes = encode_message(
            WireKind::TerminalAttachRequest,
            request_id,
            u32::try_from(DEFAULT_DEADLINE.as_millis()).unwrap_or(u32::MAX),
            &request,
        )
        .map_err(protocol_error)?;
        let mut transport = tokio::time::timeout_at(
            deadline,
            AttachmentTransport::open(&self.socket, self.target),
        )
        .await
        .map_err(|_| super::control_timeout())??;
        transport.write_until(&bytes, deadline).await?;

        tokio::time::timeout_at(deadline, async {
            let mut paths = Vec::new();
            loop {
                let frame = match transport.read_item().await? {
                    AttachmentTransportItem::Path(path) => {
                        check_initial_capacity(paths.len())?;
                        paths.push(path);
                        continue;
                    }
                    AttachmentTransportItem::Session(frame) => frame,
                };
                if frame.request_id != request_id {
                    return Err(malformed(
                        "remote resume initial update correlation mismatch",
                    ));
                }
                if frame.kind == WireKind::ServiceErrorResponse {
                    return Err(service_error(&frame)?);
                }
                let (attachment_id, initial, terminal_size) = match frame.kind {
                    WireKind::TerminalSemanticSnapshot => {
                        let snapshot: v2::TerminalSemanticSnapshot = frame
                            .decode_message(WireKind::TerminalSemanticSnapshot)
                            .map_err(protocol_error)?;
                        let (session_id, attachment_id, snapshot) =
                            zterm_proto::terminal_surface_snapshot_from_message(snapshot)
                                .map_err(protocol_error)?;
                        if session_id != self.session_id {
                            return Err(malformed(
                                "remote resume snapshot changed the frozen session identity",
                            ));
                        }
                        let size = snapshot.surface.size;
                        (
                            attachment_id,
                            LocalAttachmentEvent::Snapshot(snapshot),
                            size,
                        )
                    }
                    WireKind::TerminalSemanticDelta => {
                        let delta: v2::TerminalSemanticDelta = frame
                            .decode_message(WireKind::TerminalSemanticDelta)
                            .map_err(protocol_error)?;
                        let (attachment_id, delta) =
                            zterm_proto::terminal_surface_delta_from_message(delta)
                                .map_err(protocol_error)?;
                        if delta.from_revision
                            != Revision::new(self.applied_revision.load(Ordering::Acquire))
                        {
                            return Err(malformed(
                                "remote resume delta does not continue the applied revision",
                            ));
                        }
                        let size = delta.size;
                        (attachment_id, LocalAttachmentEvent::Delta(delta), size)
                    }
                    _ => {
                        return Err(malformed(
                            "remote resume began with an invalid Session update",
                        ));
                    }
                };
                return Ok(ReconnectedAttachment {
                    transport,
                    attachment_id,
                    initial,
                    paths,
                    terminal_size,
                });
            }
        })
        .await
        .map_err(|_| {
            DaemonError::new(
                DomainErrorKind::DeadlineExceeded,
                "timed out waiting for remote resume state",
            )
        })?
    }

    fn enter_reconnecting(&mut self) {
        if self.reconnect_pending {
            return;
        }
        self.transport.invalidate();
        self.write_error = Some(super::attachment_cancelled());
        let history_gap = self.pending_history_window.take().map(|(_, query)| {
            reconnect_history_gap(query, self.applied_revision.load(Ordering::Acquire))
        });
        self.reconnect_pending = true;
        self.pending_transport_events.clear();
        self.deferred.clear();
        self.operation_lease = None;
        self.next_operation_sequence = 1;
        self.pending_takeover_request_id = None;
        if let Some(gap) = history_gap {
            self.pending_transport_events
                .push_back(LocalAttachmentEvent::HistoryWindow(gap));
        }
        self.pending_transport_events
            .push_back(LocalAttachmentEvent::TransportState(
                terminal_transport_state(
                    self.attachment_id,
                    v2::TerminalTransportState::Reconnecting,
                ),
            ));
    }

    /// Returns the attached daemon-lifetime session identity.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Reports whether this frontend is waiting to establish and acknowledge a
    /// replacement remote stream epoch.
    #[must_use]
    pub(crate) const fn reconnect_pending(&self) -> bool {
        self.reconnect_pending
    }

    /// Installs the lifecycle-locked recovery capability for a remote view.
    ///
    /// A local view deliberately ignores this hook because its target Session
    /// disappears with the stopped daemon and therefore cannot be resumed.
    pub(crate) fn set_remote_daemon_restarter(
        &mut self,
        restarter: Arc<dyn RemoteDaemonRestarter>,
    ) {
        if self.target.device_id().is_some() {
            self.remote_daemon_restarter = Some(restarter);
        }
    }

    /// Returns this socket view's attachment identity.
    #[must_use]
    pub const fn attachment_id(&self) -> AttachmentId {
        self.attachment_id
    }

    /// Whether this frontend Session client uses an opaque remote tunnel.
    #[must_use]
    pub const fn is_remote(&self) -> bool {
        self.target.device_id().is_some()
    }

    /// Shared frontend-applied revision observed by reconnect establishment.
    pub(crate) fn applied_revision_tracker(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.applied_revision)
    }

    /// Transfers the initial full state to the prepared view exactly once.
    #[must_use]
    pub fn take_initial_snapshot(&mut self) -> Option<TerminalSurfaceSnapshot> {
        self.initial_snapshot.take()
    }

    #[cfg(test)]
    pub(crate) fn terminal_driver_test_pair(
        target: ResolvedSessionTarget,
        session_id: SessionId,
        attachment_id: AttachmentId,
    ) -> (Self, tokio::net::UnixStream) {
        let (stream, peer) =
            tokio::net::UnixStream::pair().expect("test-private terminal driver Unix stream pair");
        (
            Self {
                transport: AttachmentTransport::Direct {
                    stream,
                    decoder: FrameDecoder::new(),
                    queued: VecDeque::new(),
                },
                write_error: None,
                takeover_deadline: None,
                deferred: VecDeque::new(),
                pending_transport_events: VecDeque::new(),
                socket: PathBuf::new(),
                session_id,
                attachment_id,
                target,
                resume_view_id: None,
                applied_revision: Arc::new(AtomicU64::new(1)),
                latest_viewport: zterm_core::terminal::TerminalSize::new(24, 80),
                reconnect_pending: false,
                force_full_sync: false,
                initial_snapshot: Some(TerminalSurfaceSnapshot {
                    revision: Revision::new(1),
                    surface: zterm_core::terminal::TerminalSurface {
                        size: zterm_core::terminal::TerminalSize::new(24, 80),
                        active_screen: zterm_core::terminal::ActiveScreen::Main,
                        rows: (0..24)
                            .map(|_| zterm_core::terminal::TerminalSurfaceRow {
                                cells: vec![zterm_core::terminal::TerminalCell::default(); 80],
                                wrapped: false,
                            })
                            .collect(),
                        cursor: zterm_core::terminal::TerminalCursor {
                            row: 0,
                            column: 0,
                            visible: true,
                            style: zterm_core::terminal::TerminalStyle::default(),
                        },
                        modes: zterm_core::terminal::TerminalModes::default(),
                        scroll_metrics: Some(zterm_core::terminal::TerminalScrollMetrics {
                            epoch: Revision::ZERO,
                            revision: Revision::new(1),
                            offset_from_bottom: 0,
                            max_offset_from_bottom: 0,
                            viewport_rows: 24,
                        }),
                    },
                }),
                terminal_rows: 24,
                next_request_id: 2,
                operation_lease: None,
                next_operation_sequence: 1,
                pending_takeover_request_id: None,
                pending_history_window: None,
                remote_daemon_restarter: None,
            },
            peer,
        )
    }

    /// Atomically acknowledges the exact full snapshot revision.
    pub async fn snapshot_applied(&mut self, revision: Revision) -> Result<(), DaemonError> {
        // This is the frontend's rendered baseline even when the following
        // transport write is interrupted. A resume may safely advertise it;
        // the target falls back to a full snapshot when its checkpoint differs.
        self.applied_revision
            .store(revision.get(), Ordering::Release);
        self.send(
            WireKind::TerminalSnapshotApplied,
            &v2::TerminalSnapshotApplied {
                attachment_id: Some(self.attachment_id.into()),
                revision: revision.get(),
            },
        )
        .await
        .map(|_| ())
    }

    /// Sends controller input without waiting for a redundant success ACK.
    pub async fn write_input(&mut self, bytes: Vec<u8>) -> Result<(), DaemonError> {
        self.send(
            WireKind::TerminalInput,
            &v2::TerminalInput {
                operation_id: None,
                attachment_id: Some(self.attachment_id.into()),
                bytes,
            },
        )
        .await
        .map(|_| ())
    }

    /// Requests one validated native/model viewport change.
    pub async fn resize(
        &mut self,
        size: zterm_core::terminal::TerminalSize,
    ) -> Result<(), DaemonError> {
        // Retain the frontend's latest desired viewport across a tunnel epoch.
        self.latest_viewport = size;
        self.send(
            WireKind::TerminalResize,
            &v2::TerminalResize {
                operation_id: None,
                attachment_id: Some(self.attachment_id.into()),
                rows: u32::from(size.rows),
                columns: u32::from(size.columns),
            },
        )
        .await
        .map(|_| ())
    }

    /// Discards the client baseline and requests a fresh snapshot.
    pub async fn request_sync(&mut self, known_revision: Revision) -> Result<(), DaemonError> {
        if self.target.device_id().is_some() {
            self.force_full_sync = true;
        }
        self.send(
            WireKind::TerminalSyncRequest,
            &v2::TerminalSyncRequest {
                attachment_id: Some(self.attachment_id.into()),
                known_revision: known_revision.get(),
            },
        )
        .await
        .map(|_| ())
    }

    /// Requests one stateless bounded history window. Exactly one such request
    /// may be outstanding per view.
    pub(crate) async fn request_history_window(
        &mut self,
        query: TerminalHistoryWindowQuery,
    ) -> Result<(), DaemonError> {
        if self.pending_history_window.is_some() {
            return Err(resource_error(
                "a terminal history window response is already pending",
            ));
        }
        if !query.is_valid() {
            return Err(resource_error(
                "terminal history window query is outside the allowed range",
            ));
        }
        let request_id = self
            .send(
                WireKind::TerminalHistoryWindowRequest,
                &v2::TerminalHistoryWindowRequest {
                    attachment_id: Some(self.attachment_id.into()),
                    anchor: Some(query.anchor.into()),
                    target_offset_from_bottom: query.target_offset_from_bottom,
                    older_margin_rows: u32::from(query.older_margin_rows),
                    newer_margin_rows: u32::from(query.newer_margin_rows),
                },
            )
            .await?;
        self.pending_history_window = Some((request_id, query));
        if self.reconnect_pending {
            self.pending_history_window = None;
            self.pending_transport_events
                .push_front(LocalAttachmentEvent::HistoryWindow(reconnect_history_gap(
                    query,
                    self.applied_revision.load(Ordering::Acquire),
                )));
        }
        Ok(())
    }

    /// Commits a previously prepared and acknowledged takeover attachment.
    pub async fn takeover(&mut self) -> Result<(), DaemonError> {
        self.begin_takeover().await.map(|_| ())
    }

    /// Sends a takeover and returns the opaque token required after ambiguous
    /// response loss.
    #[doc(hidden)]
    pub async fn begin_takeover(&mut self) -> Result<LocalTakeoverRetryToken, DaemonError> {
        if self.pending_takeover_request_id.is_some() {
            return Err(malformed("a takeover response is already pending"));
        }
        let deadline = super::control_deadline();
        let operation_id = match tokio::time::timeout_at(deadline, self.next_operation_id()).await {
            Ok(result) => result?,
            Err(_) => {
                self.invalidate_transport();
                return Err(super::control_timeout());
            }
        };
        let receipt = LocalTakeoverRetryToken {
            operation_id,
            session_id: self.session_id,
        };
        self.takeover_deadline = Some(deadline);
        self.pending_takeover_request_id = Some(self.send_takeover(operation_id).await?);
        Ok(receipt)
    }

    /// Continues an ambiguously completed takeover on a newly synchronized
    /// attachment without inventing a new logical operation.
    #[doc(hidden)]
    pub async fn retry_takeover(
        &mut self,
        token: LocalTakeoverRetryToken,
    ) -> Result<(), DaemonError> {
        if token.session_id != self.session_id {
            return Err(malformed("takeover retry token belongs to another session"));
        }
        if self.pending_takeover_request_id.is_some() {
            return Err(malformed("a takeover response is already pending"));
        }
        self.takeover_deadline = Some(super::control_deadline());
        self.pending_takeover_request_id = Some(self.send_takeover(token.operation_id).await?);
        Ok(())
    }

    async fn send_takeover(&mut self, operation_id: OperationId) -> Result<u64, DaemonError> {
        let deadline = self
            .takeover_deadline
            .unwrap_or_else(super::control_deadline);
        // Expiry before write is definitive; once polling a write, its outcome may be unknown.
        if deadline <= tokio::time::Instant::now() {
            return Err(super::control_timeout());
        }
        let result = tokio::time::timeout_at(
            deadline,
            self.send(
                WireKind::SessionTakeoverRequest,
                &v2::SessionTakeoverRequest {
                    operation_id: Some(operation_id.into()),
                    target: Some(resolved_target_wire(self.target)),
                    session_id: Some(self.session_id.into()),
                    attachment_id: Some(self.attachment_id.into()),
                },
            ),
        )
        .await;
        match result {
            Ok(Ok(request_id)) => Ok(request_id),
            Ok(Err(error)) if !is_retryable_remote_reconnect(error.kind()) => Err(error),
            Ok(Err(_)) | Err(_) => {
                self.invalidate_transport();
                Err(takeover_outcome_unknown())
            }
        }
    }

    /// Reads one typed terminal event, bounded by the caller's deadline.
    pub async fn read_event(
        &mut self,
        deadline: Duration,
    ) -> Result<LocalAttachmentEvent, DaemonError> {
        tokio::time::timeout(deadline, self.read_next_event())
            .await
            .map_err(|_| {
                DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    "timed out waiting for local terminal event",
                )
            })?
    }

    /// Reads the next event without imposing a timeout on an active view.
    ///
    /// The owner may cancel and repoll this future: decoder and queued-byte
    /// state stay in this same client object.
    pub(crate) async fn read_next_event(&mut self) -> Result<LocalAttachmentEvent, DaemonError> {
        if self.pending_takeover_request_id.is_some()
            && let Some(deadline) = self.takeover_deadline
        {
            if deadline > tokio::time::Instant::now()
                && let Ok(result) =
                    tokio::time::timeout_at(deadline, self.read_next_event_inner()).await
                && !result
                    .as_ref()
                    .err()
                    .is_some_and(|error| is_retryable_remote_reconnect(error.kind()))
            {
                return result;
            }
            self.invalidate_transport();
            return Err(takeover_outcome_unknown());
        }
        self.read_next_event_inner().await
    }

    async fn read_next_event_inner(&mut self) -> Result<LocalAttachmentEvent, DaemonError> {
        if let Some(event) = self.pending_transport_events.pop_front() {
            return Ok(event);
        }
        if self.reconnect_pending {
            self.reconnect_remote().await?;
            return self
                .pending_transport_events
                .pop_front()
                .ok_or_else(|| malformed("remote resume omitted synchronization state"));
        }
        let frame = if let Some(frame) = self.deferred.pop_front() {
            frame
        } else {
            let item = match self.transport.read_item().await {
                Ok(item) => item,
                Err(error)
                    if self.target.device_id().is_some()
                        && is_retryable_remote_reconnect(error.kind()) =>
                {
                    if self.pending_takeover_request_id.is_some() {
                        self.pending_takeover_request_id = None;
                        self.operation_lease = None;
                        self.next_operation_sequence = 1;
                        return Err(DaemonError::new(
                            DomainErrorKind::OperationOutcomeUnknown,
                            "terminal takeover may have committed before tunnel loss",
                        ));
                    }
                    self.enter_reconnecting();
                    return self
                        .pending_transport_events
                        .pop_front()
                        .ok_or_else(|| malformed("remote reconnect omitted its transition event"));
                }
                Err(error) => return Err(error),
            };
            match item {
                AttachmentTransportItem::Session(frame) => frame,
                AttachmentTransportItem::Path(path) => {
                    return Ok(LocalAttachmentEvent::ConnectionStatus(
                        v2::TerminalConnectionStatusEvent {
                            attachment_id: Some(self.attachment_id.into()),
                            path: path.path,
                            rtt_ms: path.rtt_ms,
                        },
                    ));
                }
            }
        };
        if frame.kind == WireKind::ServiceErrorResponse {
            let error = service_error(&frame)?;
            if self.pending_takeover_request_id == Some(frame.request_id) {
                self.pending_takeover_request_id = None;
            }
            if self
                .pending_history_window
                .is_some_and(|(request_id, _)| request_id == frame.request_id)
            {
                self.pending_history_window = None;
            }
            if error.kind() == DomainErrorKind::OperationOutcomeUnknown {
                self.operation_lease = None;
                self.next_operation_sequence = 1;
            }
            return Err(error);
        }
        match frame.kind {
            WireKind::TerminalTransportStateEvent => {
                if self.target.device_id().is_some() {
                    return Err(malformed(
                        "remote target Session stream carried a same-UID-only transport state",
                    ));
                }
                let state: v2::TerminalTransportStateEvent = frame
                    .decode_message(WireKind::TerminalTransportStateEvent)
                    .map_err(protocol_error)?;
                self.require_attachment(state.attachment_id.clone())?;
                v2::TerminalTransportState::try_from(state.state)
                    .map_err(|_| malformed("unknown terminal transport state"))?;
                Ok(LocalAttachmentEvent::TransportState(state))
            }
            WireKind::TerminalConnectionStatusEvent => {
                if self.target.device_id().is_some() {
                    return Err(malformed(
                        "remote target Session stream carried a same-UID-only connection status",
                    ));
                }
                let status: v2::TerminalConnectionStatusEvent = frame
                    .decode_message(WireKind::TerminalConnectionStatusEvent)
                    .map_err(protocol_error)?;
                self.require_attachment(status.attachment_id.clone())?;
                match v2::TerminalConnectionPath::try_from(status.path) {
                    Ok(v2::TerminalConnectionPath::Unknown)
                    | Ok(v2::TerminalConnectionPath::Direct)
                    | Ok(v2::TerminalConnectionPath::Relay) => {}
                    Ok(v2::TerminalConnectionPath::Unspecified) | Err(_) => {
                        return Err(malformed("unknown terminal connection path"));
                    }
                }
                Ok(LocalAttachmentEvent::ConnectionStatus(status))
            }
            WireKind::TerminalSemanticSnapshot => {
                let snapshot: v2::TerminalSemanticSnapshot = frame
                    .decode_message(WireKind::TerminalSemanticSnapshot)
                    .map_err(protocol_error)?;
                let (session_id, attachment_id, surface) =
                    zterm_proto::terminal_surface_snapshot_from_message(snapshot)
                        .map_err(protocol_error)?;
                if session_id != self.session_id || attachment_id != self.attachment_id {
                    return Err(malformed("semantic terminal snapshot identity mismatch"));
                }
                self.force_full_sync = false;
                self.terminal_rows = u32::from(surface.surface.size.rows);
                Ok(LocalAttachmentEvent::Snapshot(surface))
            }
            WireKind::TerminalSemanticDelta => {
                let delta: v2::TerminalSemanticDelta = frame
                    .decode_message(WireKind::TerminalSemanticDelta)
                    .map_err(protocol_error)?;
                let (attachment_id, semantic) =
                    zterm_proto::terminal_surface_delta_from_message(delta)
                        .map_err(protocol_error)?;
                if attachment_id != self.attachment_id {
                    return Err(malformed("semantic terminal delta attachment_id mismatch"));
                }
                self.terminal_rows = u32::from(semantic.size.rows);
                Ok(LocalAttachmentEvent::Delta(semantic))
            }
            WireKind::TerminalSemanticHistoryWindowFrame => {
                let Some((request_id, query)) = self.pending_history_window else {
                    return Err(malformed(
                        "semantic terminal history window correlation mismatch",
                    ));
                };
                if request_id != frame.request_id {
                    return Err(malformed(
                        "semantic terminal history window correlation mismatch",
                    ));
                }
                let window: v2::TerminalSemanticHistoryWindowFrame = frame
                    .decode_message(WireKind::TerminalSemanticHistoryWindowFrame)
                    .map_err(protocol_error)?;
                let (attachment_id, result) =
                    zterm_proto::terminal_surface_history_window_from_message(window, query)
                        .map_err(protocol_error)?;
                if attachment_id != self.attachment_id {
                    return Err(malformed(
                        "semantic terminal history window attachment_id mismatch",
                    ));
                }
                self.pending_history_window = None;
                Ok(LocalAttachmentEvent::HistoryWindow(result))
            }
            WireKind::TerminalClipboardWrite => {
                if frame.request_id != 0 {
                    return Err(malformed(
                        "terminal clipboard effect must use request_id zero",
                    ));
                }
                let message: v2::TerminalClipboardWrite = frame
                    .decode_message(WireKind::TerminalClipboardWrite)
                    .map_err(protocol_error)?;
                let (attachment_id, write) =
                    zterm_proto::terminal_clipboard_write_from_message(message)
                        .map_err(protocol_error)?;
                if attachment_id != self.attachment_id {
                    return Err(malformed(
                        "terminal clipboard effect attachment_id mismatch",
                    ));
                }
                Ok(LocalAttachmentEvent::ClipboardWrite(write))
            }
            WireKind::TerminalSyncRequired => {
                let required: v2::TerminalSyncRequired = frame
                    .decode_message(WireKind::TerminalSyncRequired)
                    .map_err(protocol_error)?;
                self.require_attachment(required.attachment_id.clone())?;
                Ok(LocalAttachmentEvent::SyncRequired(required))
            }
            WireKind::SessionMutateResponse => {
                if self.pending_takeover_request_id != Some(frame.request_id) {
                    return Err(malformed("takeover response correlation mismatch"));
                }
                self.pending_takeover_request_id = None;
                Ok(LocalAttachmentEvent::Takeover(mutate_response(frame)?))
            }
            WireKind::TerminalLeaseLost => {
                let lost: v2::TerminalLeaseLost = frame
                    .decode_message(WireKind::TerminalLeaseLost)
                    .map_err(protocol_error)?;
                self.require_attachment(lost.attachment_id.clone())?;
                Ok(LocalAttachmentEvent::LeaseLost(lost))
            }
            WireKind::TerminalSessionEnded => {
                let ended: v2::TerminalSessionEnded = frame
                    .decode_message(WireKind::TerminalSessionEnded)
                    .map_err(protocol_error)?;
                self.require_attachment(ended.attachment_id.clone())?;
                let session_id: SessionId = ended
                    .session_id
                    .clone()
                    .ok_or_else(|| malformed("session-ended event omitted session_id"))?
                    .try_into()
                    .map_err(protocol_error)?;
                if session_id != self.session_id {
                    return Err(malformed("session-ended event session_id mismatch"));
                }
                Ok(LocalAttachmentEvent::SessionEnded(ended))
            }
            kind => Err(malformed(format!(
                "wire kind {kind:?} is invalid from a terminal attachment"
            ))),
        }
    }

    /// Detaches this view while leaving the session and PTY running.
    pub async fn detach(&mut self) -> Result<(), DaemonError> {
        self.send(
            WireKind::TerminalDetach,
            &v2::TerminalDetach {
                attachment_id: Some(self.attachment_id.into()),
            },
        )
        .await?;
        if self.reconnect_pending {
            return Ok(());
        }
        self.transport.shutdown().await
    }

    async fn send<Message: prost::Message>(
        &mut self,
        kind: WireKind,
        message: &Message,
    ) -> Result<u64, DaemonError> {
        let request_id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or_else(|| resource_error("local attachment request ID exhausted"))?;
        let bytes = encode_message(kind, request_id, 0, message).map_err(protocol_error)?;
        let result = match &self.write_error {
            Some(error) => Err(error.clone()),
            None => self.transport.write_session_bytes(&bytes).await,
        };
        if let Err(error) = result {
            self.write_error = Some(error.clone());
            if self.target.device_id().is_some() && is_retryable_remote_reconnect(error.kind()) {
                self.enter_reconnecting();
                return match kind {
                    // These messages are never replayed. The frontend state
                    // retained above is enough to converge after resume;
                    // input at the exact loss boundary remains intentionally
                    // best-effort, matching an interrupted terminal stream.
                    WireKind::TerminalInput
                    | WireKind::TerminalResize
                    | WireKind::TerminalSnapshotApplied
                    | WireKind::TerminalSyncRequest
                    | WireKind::TerminalHistoryWindowRequest
                    | WireKind::TerminalDetach => Ok(request_id),
                    // A takeover may have committed at the target. Never mint
                    // a fresh logical operation or report a false failure.
                    WireKind::SessionTakeoverRequest => Err(DaemonError::new(
                        DomainErrorKind::OperationOutcomeUnknown,
                        "terminal takeover may have committed before tunnel loss",
                    )),
                    _ => Err(error),
                };
            }
            return Err(error);
        }
        Ok(request_id)
    }

    pub(super) fn invalidate_transport(&mut self) {
        self.transport.invalidate();
        self.write_error = Some(super::attachment_cancelled());
        self.deferred.clear();
        self.pending_transport_events.clear();
    }

    fn reserve_deferred(&mut self, bytes: usize) -> Result<(), DaemonError> {
        let retained = self
            .deferred
            .iter()
            .map(|frame| frame.payload.len())
            .sum::<usize>();
        // Sideband statuses have fixed identity and scalar fields, bounded by 64 bytes.
        let sideband_bytes = self.pending_transport_events.len() * 64;
        if self.deferred.len() + self.pending_transport_events.len() >= MAX_DEFERRED_FRAMES
            || retained + sideband_bytes + bytes > zterm_proto::MAX_FRAME_BYTES
        {
            self.invalidate_transport();
            return Err(resource_error(
                "terminal control response buffer is exhausted",
            ));
        }
        Ok(())
    }

    async fn read_transport_frame(&mut self) -> Result<DecodedFrame, DaemonError> {
        loop {
            match self.transport.read_item().await? {
                AttachmentTransportItem::Session(frame) => return Ok(frame),
                AttachmentTransportItem::Path(path) => {
                    self.reserve_deferred(64)?;
                    self.pending_transport_events.push_back(
                        LocalAttachmentEvent::ConnectionStatus(v2::TerminalConnectionStatusEvent {
                            attachment_id: Some(self.attachment_id.into()),
                            path: path.path,
                            rtt_ms: path.rtt_ms,
                        }),
                    );
                }
            }
        }
    }

    async fn next_operation_id(&mut self) -> Result<OperationId, DaemonError> {
        if self.operation_lease.is_none() {
            let request_id = self.next_request_id;
            self.send(
                WireKind::SessionOperationLeaseRequest,
                &v2::SessionOperationLeaseRequest {
                    target: Some(resolved_target_wire(self.target)),
                },
            )
            .await?;
            loop {
                let frame = self.read_transport_frame().await?;
                if frame.request_id != request_id {
                    self.reserve_deferred(frame.payload.len())?;
                    self.deferred.push_back(frame);
                    continue;
                }
                if frame.kind == WireKind::ServiceErrorResponse {
                    return Err(service_error(&frame)?);
                }
                if frame.kind != WireKind::SessionOperationLeaseResponse {
                    return Err(malformed("operation lease response kind mismatch"));
                }
                let response: v2::SessionOperationLeaseResponse = decode_response(&frame)?;
                self.operation_lease = Some(
                    response
                        .lease
                        .ok_or_else(|| malformed("operation lease response omitted lease"))?
                        .try_into()
                        .map_err(protocol_error)?,
                );
                break;
            }
        }
        let sequence = self.next_operation_sequence;
        self.next_operation_sequence = sequence.checked_add(1).ok_or_else(|| {
            self.operation_lease = None;
            self.next_operation_sequence = 1;
            resource_error("local attachment operation sequence exhausted")
        })?;
        Ok(OperationId {
            lease: self.operation_lease.expect("lease was allocated above"),
            sequence,
        })
    }

    fn require_attachment(
        &self,
        attachment_id: Option<v2::AttachmentId>,
    ) -> Result<(), DaemonError> {
        let attachment_id: AttachmentId = attachment_id
            .ok_or_else(|| malformed("terminal event omitted attachment_id"))?
            .try_into()
            .map_err(protocol_error)?;
        if attachment_id == self.attachment_id {
            Ok(())
        } else {
            Err(malformed("terminal event attachment_id mismatch"))
        }
    }
}

fn check_initial_capacity(retained: usize) -> Result<(), DaemonError> {
    if retained >= MAX_DEFERRED_FRAMES {
        Err(resource_error(
            "terminal handshake metadata buffer is exhausted",
        ))
    } else {
        Ok(())
    }
}

fn takeover_outcome_unknown() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::OperationOutcomeUnknown,
        "terminal takeover may have committed before its response was received",
    )
}

fn random_resume_view_id() -> Result<ResumeViewId, DaemonError> {
    let mut bytes = [0_u8; ResumeViewId::LENGTH];
    SystemRandom::new().fill(&mut bytes).map_err(|_| {
        DaemonError::new(
            DomainErrorKind::TransportUnavailable,
            "operating-system randomness is unavailable for a terminal view",
        )
    })?;
    Ok(ResumeViewId::from_array(bytes))
}

fn reconnect_history_gap(
    query: TerminalHistoryWindowQuery,
    applied_revision: u64,
) -> TerminalSurfaceHistoryWindowResult {
    TerminalSurfaceHistoryWindowResult::HistoryGap {
        epoch: query.anchor.epoch,
        revision: Revision::new(applied_revision.max(query.anchor.revision.get()).max(1)),
    }
}

fn terminal_transport_state(
    attachment_id: AttachmentId,
    state: v2::TerminalTransportState,
) -> v2::TerminalTransportStateEvent {
    v2::TerminalTransportStateEvent {
        attachment_id: Some(attachment_id.into()),
        state: state as i32,
    }
}

fn changed_tunnel_path_events_after_unknown(
    attachment_id: AttachmentId,
    paths: impl IntoIterator<Item = v2::LocalSessionTunnelPath>,
) -> VecDeque<LocalAttachmentEvent> {
    let mut last_path = (
        v2::TerminalConnectionPath::Unknown as i32,
        Option::<u32>::None,
    );
    paths
        .into_iter()
        .filter_map(|path| {
            let sample = (path.path, path.rtt_ms);
            if sample == last_path {
                return None;
            }
            last_path = sample;
            Some(LocalAttachmentEvent::ConnectionStatus(
                v2::TerminalConnectionStatusEvent {
                    attachment_id: Some(attachment_id.into()),
                    path: sample.0,
                    rtt_ms: sample.1,
                },
            ))
        })
        .collect()
}

const fn is_retryable_remote_reconnect(kind: DomainErrorKind) -> bool {
    matches!(
        kind,
        DomainErrorKind::AddressUnavailable
            | DomainErrorKind::TransportUnavailable
            | DomainErrorKind::DeadlineExceeded
            | DomainErrorKind::DaemonStopped
            | DomainErrorKind::Cancelled
            | DomainErrorKind::SessionOccupied
    )
}

fn create_main_outcome_unknown() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::OperationOutcomeUnknown,
        "the default Session may have been created, but no complete correlated initial attachment result was received",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ipc::resolved_target_from_wire;
    use crate::client::transport::{read_frame_parts, read_tunnel_first};
    use crate::session_wire::read_first;
    use std::sync::Mutex as StdMutex;
    use tokio::io::AsyncWriteExt;
    #[test]
    fn takeover_token_debug_redacts_operation_and_session_identity() {
        let retry_owner = LocalTakeoverRetryToken {
            operation_id: OperationId {
                lease: OperationLease {
                    daemon_incarnation: zterm_core::DaemonIncarnation::from_array(
                        *b"TAKEOVER_TOKEN_1",
                    ),
                    ordinal: 8_675_309,
                },
                sequence: 2_434_117,
            },
            session_id: SessionId::from_array(*b"SESSION_TOKEN_01"),
        };
        assert_eq!(
            format!("{retry_owner:?}"),
            "LocalTakeoverRetryToken([REDACTED])"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn silent_lease_expires_but_an_idle_attachment_has_no_control_deadline() {
        let (mut client, peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            SessionId::from_array([1; 16]),
            AttachmentId::from_array([2; 16]),
        );
        let started = tokio::time::Instant::now();
        assert_eq!(
            client
                .begin_takeover()
                .await
                .expect_err("silent lease must expire")
                .kind(),
            DomainErrorKind::DeadlineExceeded
        );
        assert_eq!(tokio::time::Instant::now() - started, DEFAULT_DEADLINE);
        assert!(matches!(client.transport, AttachmentTransport::Closed));
        assert!(
            client.pending_takeover_request_id.is_none(),
            "no takeover was submitted"
        );
        drop(peer);

        let (mut idle, _peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            SessionId::from_array([1; 16]),
            AttachmentId::from_array([2; 16]),
        );
        assert!(
            tokio::time::timeout(DEFAULT_DEADLINE * 2, idle.read_next_event())
                .await
                .is_err()
        );
        assert!(!matches!(idle.transport, AttachmentTransport::Closed));
    }

    #[tokio::test]
    async fn lease_wait_bounds_unrelated_frames_by_count_and_bytes() {
        for (count, payload_bytes) in [(9, 16), (2, 5 * 1024 * 1024)] {
            let (mut client, mut peer) = SessionClient::terminal_driver_test_pair(
                ResolvedSessionTarget::local(),
                SessionId::from_array([1; 16]),
                AttachmentId::from_array([2; 16]),
            );
            let sender = tokio::spawn(async move {
                // The lease waiter must bound envelopes before retaining or interpreting
                // any unsolicited content, including invalid content from a faulty peer.
                let frame = encode_message(
                    WireKind::TerminalSemanticSnapshot,
                    0,
                    0,
                    &v2::TerminalInput {
                        attachment_id: None,
                        bytes: vec![b'x'; payload_bytes],
                        ..v2::TerminalInput::default()
                    },
                )
                .expect("bounded content envelope");
                for _ in 0..count {
                    if peer.write_all(&frame).await.is_err() {
                        break;
                    }
                }
                peer
            });
            let error = client
                .begin_takeover()
                .await
                .expect_err("unrelated content must stay bounded");
            assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);
            assert!(matches!(client.transport, AttachmentTransport::Closed));
            assert!(client.deferred.is_empty());
            drop(sender.await.expect("bounded sender"));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn blocked_partial_write_expires_and_its_epoch_is_never_reused() {
        use tokio::io::AsyncReadExt;
        let (mut client, mut peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            SessionId::from_array([1; 16]),
            AttachmentId::from_array([2; 16]),
        );
        let error = client
            .write_input(vec![b'x'; 900_000])
            .await
            .expect_err("non-reading peer must not hold the owner");
        assert_eq!(error.kind(), DomainErrorKind::DeadlineExceeded);
        assert!(matches!(client.transport, AttachmentTransport::Closed));
        assert_eq!(
            client
                .write_input(b"later".to_vec())
                .await
                .expect_err("failed epoch cannot write again"),
            error
        );
        let mut received = Vec::new();
        peer.read_to_end(&mut received)
            .await
            .expect("timed-out epoch releases socket");
        assert!(!received.is_empty());
        assert!(
            received.len() < 900_000,
            "fixture must interrupt a partial frame"
        );
        let mut decoder = FrameDecoder::new();
        assert!(
            decoder
                .feed(&received)
                .expect("valid frame prefix")
                .is_empty()
        );
        assert!(
            decoder.finish().is_err(),
            "no later command repairs or follows the partial frame"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn unanswered_sent_takeover_expires_as_unknown_without_replay() {
        let (mut client, _peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            SessionId::from_array([1; 16]),
            AttachmentId::from_array([2; 16]),
        );
        client.operation_lease = Some(OperationLease {
            daemon_incarnation: zterm_core::DaemonIncarnation::from_array([3; 16]),
            ordinal: 1,
        });
        client
            .begin_takeover()
            .await
            .expect("takeover was submitted");
        assert_eq!(
            client
                .read_next_event()
                .await
                .expect_err("missing takeover response is ambiguous")
                .kind(),
            DomainErrorKind::OperationOutcomeUnknown
        );
        assert!(matches!(client.transport, AttachmentTransport::Closed));
        assert!(!client.reconnect_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_a_partial_write_drops_the_epoch_before_another_command() {
        use tokio::io::AsyncReadExt;
        let (mut client, mut peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            SessionId::from_array([1; 16]),
            AttachmentId::from_array([2; 16]),
        );
        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                client.write_input(vec![b'x'; 900_000])
            )
            .await
            .is_err()
        );
        assert!(matches!(client.transport, AttachmentTransport::Closed));
        assert!(client.write_input(b"later".to_vec()).await.is_err());
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes)
            .await
            .expect("cancelled write releases socket");
        assert!(!bytes.is_empty() && bytes.len() < 900_000);
    }

    #[tokio::test]
    async fn lease_wait_also_bounds_tunnel_path_sidebands() {
        let (mut client, mut peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(DeviceId::from_array([3; 32])),
            SessionId::from_array([1; 16]),
            AttachmentId::from_array([2; 16]),
        );
        let AttachmentTransport::Direct { stream, .. } =
            std::mem::replace(&mut client.transport, AttachmentTransport::Closed)
        else {
            panic!("fixture is direct");
        };
        client.transport = tunnel_transport(stream);
        for rtt_ms in 0..9 {
            peer.write_all(&tunnel_envelope(
                WireKind::LocalSessionTunnelPath,
                &v2::LocalSessionTunnelPath {
                    path: v2::TerminalConnectionPath::Direct as i32,
                    rtt_ms: Some(rtt_ms),
                },
            ))
            .await
            .expect("bounded sideband fixture");
        }
        assert_eq!(
            client
                .begin_takeover()
                .await
                .expect_err("sidebands must not grow while awaiting lease")
                .kind(),
            DomainErrorKind::ResourceExhausted
        );
        assert!(matches!(client.transport, AttachmentTransport::Closed));
        assert!(client.pending_transport_events.is_empty());
    }

    struct FailingRemoteDaemonRestarter {
        calls: Arc<AtomicU64>,
    }

    impl RemoteDaemonRestarter for FailingRemoteDaemonRestarter {
        fn ensure_running<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'a>> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            Box::pin(async {
                Err(DaemonError::new(
                    DomainErrorKind::DaemonStartTimeout,
                    "injected viewer-daemon restart failure",
                ))
            })
        }
    }

    struct AcceptingRemoteDaemonRestarter {
        socket: PathBuf,
        target: DeviceId,
        new_attachment: AttachmentId,
        calls: Arc<AtomicU64>,
        server: StdMutex<Option<tokio::task::JoinHandle<v2::TerminalAttachRequest>>>,
    }

    impl RemoteDaemonRestarter for AcceptingRemoteDaemonRestarter {
        fn ensure_running<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'a>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::AcqRel);
                let listener = tokio::net::UnixListener::bind(&self.socket).map_err(|error| {
                    DaemonError::new(
                        DomainErrorKind::DaemonStartTimeout,
                        format!("unable to bind replacement viewer-daemon fixture: {error}"),
                    )
                })?;
                let target = self.target;
                let new_attachment = self.new_attachment;
                let task = tokio::spawn(async move {
                    serve_replacement_tunnel_once(listener, target, new_attachment).await
                });
                let mut server = self.server.lock().map_err(|_| {
                    DaemonError::new(
                        DomainErrorKind::IdentityStateMismatch,
                        "replacement viewer-daemon fixture task lock is poisoned",
                    )
                })?;
                if server.replace(task).is_some() {
                    return Err(DaemonError::new(
                        DomainErrorKind::IdentityStateMismatch,
                        "replacement viewer-daemon fixture was started more than once",
                    ));
                }
                Ok(())
            })
        }
    }

    fn semantic_snapshot(
        session_id: SessionId,
        attachment_id: AttachmentId,
    ) -> v2::TerminalSemanticSnapshot {
        zterm_proto::terminal_surface_snapshot_message(
            session_id,
            attachment_id,
            TerminalSurfaceSnapshot {
                revision: Revision::new(1),
                surface: zterm_core::terminal::TerminalSurface {
                    size: zterm_core::terminal::TerminalSize::new(24, 80),
                    active_screen: zterm_core::terminal::ActiveScreen::Main,
                    rows: (0..24)
                        .map(|_| zterm_core::terminal::TerminalSurfaceRow {
                            cells: vec![zterm_core::terminal::TerminalCell::default(); 80],
                            wrapped: false,
                        })
                        .collect(),
                    cursor: zterm_core::terminal::TerminalCursor {
                        row: 0,
                        column: 0,
                        visible: true,
                        style: Default::default(),
                    },
                    modes: zterm_core::terminal::TerminalModes::default(),
                    scroll_metrics: Some(zterm_core::terminal::TerminalScrollMetrics {
                        epoch: Revision::ZERO,
                        revision: Revision::new(1),
                        offset_from_bottom: 0,
                        max_offset_from_bottom: 0,
                        viewport_rows: 24,
                    }),
                },
            },
        )
    }

    fn tunnel_transport(stream: tokio::net::UnixStream) -> AttachmentTransport {
        AttachmentTransport::Tunnel {
            stream,
            envelope_decoder: FrameDecoder::new(),
            queued_envelopes: VecDeque::new(),
            session_decoder: FrameDecoder::new(),
            queued_session_frames: VecDeque::new(),
            remote_half_closed: false,
        }
    }

    fn tunnel_envelope<Message: prost::Message>(kind: WireKind, message: &Message) -> Vec<u8> {
        encode_message(kind, 0, 0, message).expect("encode bounded tunnel envelope")
    }

    async fn read_tunneled_session_command(
        stream: &mut tokio::net::UnixStream,
        decoder: &mut FrameDecoder,
        queued: &mut VecDeque<DecodedFrame>,
    ) -> DecodedFrame {
        let envelope = read_frame_parts(stream, decoder, queued)
            .await
            .expect("read frontend tunnel envelope");
        assert_eq!(envelope.kind, WireKind::LocalSessionTunnelData);
        assert_eq!(envelope.request_id, 0);
        assert_eq!(envelope.deadline_ms, 0);
        let data: v2::LocalSessionTunnelData = envelope
            .decode_message(WireKind::LocalSessionTunnelData)
            .expect("decode frontend tunnel Data");
        let mut session_decoder = FrameDecoder::new();
        let mut frames = session_decoder
            .feed(&data.bytes)
            .expect("decode frontend Session command");
        assert_eq!(frames.len(), 1);
        session_decoder
            .finish()
            .expect("frontend Data contains one complete Session command");
        frames.remove(0)
    }

    async fn serve_replacement_tunnel_once(
        listener: tokio::net::UnixListener,
        target: DeviceId,
        new_attachment: AttachmentId,
    ) -> v2::TerminalAttachRequest {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("replacement viewer daemon accepts one tunnel");
        serve_replacement_tunnel_stream(&mut stream, target, new_attachment).await
    }

    async fn serve_replacement_tunnel_stream(
        stream: &mut tokio::net::UnixStream,
        target: DeviceId,
        new_attachment: AttachmentId,
    ) -> v2::TerminalAttachRequest {
        let first = read_first(stream)
            .await
            .expect("replacement viewer daemon reads tunnel Open");
        assert_eq!(first.frame.kind, WireKind::LocalSessionTunnelOpenRequest);
        let open: v2::LocalSessionTunnelOpenRequest = first
            .frame
            .decode_message(WireKind::LocalSessionTunnelOpenRequest)
            .expect("decode replacement tunnel Open");
        assert_eq!(
            DeviceId::try_from(open.target_device_id.expect("replacement Open target"))
                .expect("valid replacement Open target"),
            target
        );
        stream
            .write_all(
                &encode_message(
                    WireKind::LocalSessionTunnelOpened,
                    first.frame.request_id,
                    0,
                    &v2::LocalSessionTunnelOpened {
                        protocol_version: zterm_proto::LOCAL_SESSION_TUNNEL_VERSION,
                    },
                )
                .expect("encode replacement tunnel Opened"),
            )
            .await
            .expect("write replacement tunnel Opened");

        let mut envelope_decoder = first.decoder;
        let mut queued_envelopes = first.queued;
        let data = read_frame_parts(stream, &mut envelope_decoder, &mut queued_envelopes)
            .await
            .expect("read replacement tunneled resume request");
        assert_eq!(data.kind, WireKind::LocalSessionTunnelData);
        let data: v2::LocalSessionTunnelData = data
            .decode_message(WireKind::LocalSessionTunnelData)
            .expect("decode replacement tunneled resume bytes");
        let mut session_decoder = FrameDecoder::new();
        let mut session_frames = session_decoder
            .feed(&data.bytes)
            .expect("decode replacement inner resume frame");
        assert_eq!(session_frames.len(), 1);
        session_decoder
            .finish()
            .expect("replacement Data contains one complete resume frame");
        let attach_frame = session_frames.remove(0);
        assert_eq!(attach_frame.kind, WireKind::TerminalAttachRequest);
        let request_id = attach_frame.request_id;
        let attach: v2::TerminalAttachRequest = attach_frame
            .decode_message(WireKind::TerminalAttachRequest)
            .expect("decode replacement resume request");
        let viewport = zterm_core::terminal::TerminalSize::try_from(
            attach
                .viewport
                .expect("replacement resume advertises its latest viewport"),
        )
        .expect("replacement resume viewport is valid");
        let known_revision = attach
            .known_revision
            .expect("replacement resume advertises an applied revision");
        let delta = TerminalSurfaceDelta {
            from_revision: Revision::new(known_revision),
            to_revision: Revision::new(
                known_revision
                    .checked_add(1)
                    .expect("replacement revision fits"),
            ),
            size: viewport,
            active_screen: zterm_core::terminal::ActiveScreen::Main,
            row_patches: Vec::new(),
            cursor: zterm_core::terminal::TerminalCursor {
                row: 0,
                column: 0,
                visible: true,
                style: Default::default(),
            },
            modes: zterm_core::terminal::TerminalModes::default(),
            scroll_metrics: Some(zterm_core::terminal::TerminalScrollMetrics {
                epoch: Revision::ZERO,
                revision: Revision::new(known_revision + 1),
                offset_from_bottom: 0,
                max_offset_from_bottom: 0,
                viewport_rows: viewport.rows,
            }),
        };
        let inner = encode_message(
            WireKind::TerminalSemanticDelta,
            request_id,
            0,
            &zterm_proto::terminal_surface_delta_message(new_attachment, delta),
        )
        .expect("encode replacement resume delta");
        stream
            .write_all(&tunnel_envelope(
                WireKind::LocalSessionTunnelData,
                &v2::LocalSessionTunnelData { bytes: inner },
            ))
            .await
            .expect("write replacement resume delta");
        attach
    }

    #[tokio::test]
    async fn tunnel_adapter_surfaces_path_before_a_split_inner_frame_and_preserves_coalescing() {
        let attachment_id = AttachmentId::from_array([0x21; AttachmentId::LENGTH]);
        let mut inner = encode_message(
            WireKind::TerminalTransportStateEvent,
            0,
            0,
            &v2::TerminalTransportStateEvent {
                attachment_id: Some(attachment_id.into()),
                state: v2::TerminalTransportState::Active as i32,
            },
        )
        .expect("encode first inner Session frame");
        inner.extend_from_slice(
            &encode_message(
                WireKind::TerminalSyncRequired,
                0,
                0,
                &v2::TerminalSyncRequired {
                    attachment_id: Some(attachment_id.into()),
                    latest_revision: 9,
                },
            )
            .expect("encode second inner Session frame"),
        );

        let (client_stream, mut daemon_stream) =
            tokio::net::UnixStream::pair().expect("create same-process tunnel fixture");
        let mut transport = tunnel_transport(client_stream);
        let mut envelopes = tunnel_envelope(
            WireKind::LocalSessionTunnelData,
            &v2::LocalSessionTunnelData {
                bytes: inner[..1].to_vec(),
            },
        );
        envelopes.extend_from_slice(&tunnel_envelope(
            WireKind::LocalSessionTunnelPath,
            &v2::LocalSessionTunnelPath {
                path: v2::TerminalConnectionPath::Direct as i32,
                rtt_ms: Some(7),
            },
        ));
        envelopes.extend_from_slice(&tunnel_envelope(
            WireKind::LocalSessionTunnelData,
            &v2::LocalSessionTunnelData {
                bytes: inner[1..].to_vec(),
            },
        ));
        daemon_stream
            .write_all(&envelopes)
            .await
            .expect("write split and coalesced tunnel envelopes");

        let AttachmentTransportItem::Path(path) = transport
            .read_item()
            .await
            .expect("path is observable without awaiting a complete inner frame")
        else {
            panic!("path sideband must surface immediately");
        };
        assert_eq!(
            v2::TerminalConnectionPath::try_from(path.path),
            Ok(v2::TerminalConnectionPath::Direct)
        );
        assert_eq!(path.rtt_ms, Some(7));

        let AttachmentTransportItem::Session(first) = transport
            .read_item()
            .await
            .expect("split inner frame is reassembled")
        else {
            panic!("expected first Session frame");
        };
        assert_eq!(first.kind, WireKind::TerminalTransportStateEvent);
        let AttachmentTransportItem::Session(second) = transport
            .read_item()
            .await
            .expect("coalesced inner frame remains queued")
        else {
            panic!("expected second Session frame");
        };
        assert_eq!(second.kind, WireKind::TerminalSyncRequired);
    }

    #[tokio::test]
    async fn tunnel_adapter_rejects_zero_data_and_incomplete_inner_eof() {
        let (client_stream, mut daemon_stream) =
            tokio::net::UnixStream::pair().expect("create zero-data tunnel fixture");
        let mut transport = tunnel_transport(client_stream);
        daemon_stream
            .write_all(&tunnel_envelope(
                WireKind::LocalSessionTunnelData,
                &v2::LocalSessionTunnelData { bytes: Vec::new() },
            ))
            .await
            .expect("write invalid zero-byte Data envelope");
        let error = match transport.read_item().await {
            Err(error) => error,
            Ok(_) => panic!("zero-byte tunnel Data is invalid"),
        };
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);

        let (client_stream, mut daemon_stream) =
            tokio::net::UnixStream::pair().expect("create incomplete-inner tunnel fixture");
        let mut transport = tunnel_transport(client_stream);
        let incomplete_inner = encode_message(
            WireKind::TerminalSyncRequired,
            0,
            0,
            &v2::TerminalSyncRequired {
                attachment_id: Some(AttachmentId::from_array([0x31; 16]).into()),
                latest_revision: 1,
            },
        )
        .expect("encode an inner frame to truncate");
        let mut envelopes = tunnel_envelope(
            WireKind::LocalSessionTunnelData,
            &v2::LocalSessionTunnelData {
                bytes: incomplete_inner[..1].to_vec(),
            },
        );
        envelopes.extend_from_slice(&tunnel_envelope(
            WireKind::LocalSessionTunnelHalfClose,
            &v2::LocalSessionTunnelHalfClose {},
        ));
        envelopes.extend_from_slice(&tunnel_envelope(
            WireKind::LocalSessionTunnelClosed,
            &v2::LocalSessionTunnelClosed {
                reason: v2::LocalSessionTunnelCloseReason::RemoteEof as i32,
            },
        ));
        daemon_stream
            .write_all(&envelopes)
            .await
            .expect("write incomplete inner stream then terminal envelopes");
        let error = match transport.read_item().await {
            Err(error) => error,
            Ok(_) => panic!("Closed must validate the complete inner Session stream"),
        };
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);

        let (client_stream, mut daemon_stream) =
            tokio::net::UnixStream::pair().expect("create interrupted-inner tunnel fixture");
        let mut transport = tunnel_transport(client_stream);
        let mut envelopes = tunnel_envelope(
            WireKind::LocalSessionTunnelData,
            &v2::LocalSessionTunnelData {
                bytes: incomplete_inner[..1].to_vec(),
            },
        );
        envelopes.extend_from_slice(&tunnel_envelope(
            WireKind::LocalSessionTunnelClosed,
            &v2::LocalSessionTunnelClosed {
                reason: v2::LocalSessionTunnelCloseReason::TransportLost as i32,
            },
        ));
        daemon_stream
            .write_all(&envelopes)
            .await
            .expect("write partial inner frame then transport loss");
        let error = match transport.read_item().await {
            Err(error) => error,
            Ok(_) => panic!("transport loss must terminate the interrupted Session epoch"),
        };
        assert_eq!(
            error.kind(),
            DomainErrorKind::TransportUnavailable,
            "partial inner bytes from a lost epoch must not mask reconnect eligibility"
        );
    }

    #[tokio::test]
    async fn tunnel_adapter_rejects_data_after_half_close() {
        let (client_stream, mut daemon_stream) =
            tokio::net::UnixStream::pair().expect("create half-close tunnel fixture");
        let mut transport = tunnel_transport(client_stream);
        let mut envelopes = tunnel_envelope(
            WireKind::LocalSessionTunnelHalfClose,
            &v2::LocalSessionTunnelHalfClose {},
        );
        envelopes.extend_from_slice(&tunnel_envelope(
            WireKind::LocalSessionTunnelData,
            &v2::LocalSessionTunnelData { bytes: vec![0x01] },
        ));
        daemon_stream
            .write_all(&envelopes)
            .await
            .expect("write Data after HalfClose");
        let error = match transport.read_item().await {
            Err(error) => error,
            Ok(_) => panic!("Data after HalfClose is a tunnel protocol violation"),
        };
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
    }

    #[tokio::test]
    async fn direct_and_tunnel_adapters_share_one_session_trace_and_command_interpreter() {
        let target_device = DeviceId::from_array([0x35; DeviceId::LENGTH]);
        let session_id = SessionId::from_array([0x36; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0x37; AttachmentId::LENGTH]);
        let (mut direct, mut direct_target) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        let (mut tunneled, stale_direct_target) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(target_device),
            session_id,
            attachment_id,
        );
        drop(stale_direct_target);
        let (tunnel_stream, mut tunnel_daemon) =
            tokio::net::UnixStream::pair().expect("create paired tunnel stream");
        tunneled.transport = tunnel_transport(tunnel_stream);
        tunneled.resume_view_id = Some(ResumeViewId::from_array([0x38; ResumeViewId::LENGTH]));

        assert!(!direct.is_remote());
        assert!(tunneled.is_remote());
        assert_eq!(direct.session_id(), tunneled.session_id());
        assert_eq!(direct.attachment_id(), tunneled.attachment_id());

        let delta = TerminalSurfaceDelta {
            from_revision: Revision::new(1),
            to_revision: Revision::new(2),
            size: zterm_core::terminal::TerminalSize::new(24, 80),
            active_screen: zterm_core::terminal::ActiveScreen::Main,
            row_patches: Vec::new(),
            cursor: zterm_core::terminal::TerminalCursor {
                row: 3,
                column: 7,
                visible: true,
                style: Default::default(),
            },
            modes: zterm_core::terminal::TerminalModes::default(),
            scroll_metrics: Some(zterm_core::terminal::TerminalScrollMetrics {
                epoch: Revision::ZERO,
                revision: Revision::new(2),
                offset_from_bottom: 0,
                max_offset_from_bottom: 4,
                viewport_rows: 24,
            }),
        };
        let clipboard = TerminalClipboardWrite::new("paired clipboard".to_owned())
            .expect("valid paired clipboard effect");
        let mut target_trace = encode_message(
            WireKind::TerminalSemanticDelta,
            0,
            0,
            &zterm_proto::terminal_surface_delta_message(attachment_id, delta.clone()),
        )
        .expect("encode paired delta");
        target_trace.extend_from_slice(
            &encode_message(
                WireKind::TerminalClipboardWrite,
                0,
                0,
                &zterm_proto::terminal_clipboard_write_message(attachment_id, clipboard),
            )
            .expect("encode paired clipboard effect"),
        );
        target_trace.extend_from_slice(
            &encode_message(
                WireKind::TerminalSyncRequired,
                0,
                0,
                &v2::TerminalSyncRequired {
                    attachment_id: Some(attachment_id.into()),
                    latest_revision: 3,
                },
            )
            .expect("encode paired sync requirement"),
        );
        direct_target
            .write_all(&target_trace)
            .await
            .expect("write direct target trace");
        tunnel_daemon
            .write_all(&tunnel_envelope(
                WireKind::LocalSessionTunnelData,
                &v2::LocalSessionTunnelData {
                    bytes: target_trace,
                },
            ))
            .await
            .expect("write tunneled target trace");

        for expected_kind in ["delta", "clipboard", "sync-required"] {
            let direct_event = direct
                .read_next_event()
                .await
                .unwrap_or_else(|error| panic!("direct {expected_kind} event failed: {error}"));
            let tunnel_event = tunneled
                .read_next_event()
                .await
                .unwrap_or_else(|error| panic!("tunnel {expected_kind} event failed: {error}"));
            assert_eq!(
                direct_event, tunnel_event,
                "the shared Session interpreter diverged for {expected_kind}"
            );
        }

        let viewport = zterm_core::terminal::TerminalSize::new(31, 97);
        direct
            .snapshot_applied(Revision::new(2))
            .await
            .expect("direct acknowledgement");
        tunneled
            .snapshot_applied(Revision::new(2))
            .await
            .expect("tunneled acknowledgement");
        direct.resize(viewport).await.expect("direct resize");
        tunneled.resize(viewport).await.expect("tunneled resize");
        direct
            .write_input(b"paired-input".to_vec())
            .await
            .expect("direct input");
        tunneled
            .write_input(b"paired-input".to_vec())
            .await
            .expect("tunneled input");
        direct
            .request_sync(Revision::new(2))
            .await
            .expect("direct sync request");
        tunneled
            .request_sync(Revision::new(2))
            .await
            .expect("tunneled sync request");

        let mut direct_decoder = FrameDecoder::new();
        let mut direct_queued = VecDeque::new();
        let mut tunnel_decoder = FrameDecoder::new();
        let mut tunnel_queued = VecDeque::new();
        for expected_kind in [
            WireKind::TerminalSnapshotApplied,
            WireKind::TerminalResize,
            WireKind::TerminalInput,
            WireKind::TerminalSyncRequest,
        ] {
            let direct_command =
                read_frame_parts(&mut direct_target, &mut direct_decoder, &mut direct_queued)
                    .await
                    .expect("read direct Session command");
            let tunnel_command = read_tunneled_session_command(
                &mut tunnel_daemon,
                &mut tunnel_decoder,
                &mut tunnel_queued,
            )
            .await;
            assert_eq!(direct_command.kind, expected_kind);
            assert_eq!(
                direct_command, tunnel_command,
                "route adapter changed the target-visible {expected_kind:?} command"
            );
        }

        assert_eq!(direct.latest_viewport, viewport);
        assert_eq!(tunneled.latest_viewport, viewport);
        assert_eq!(direct.terminal_rows, tunneled.terminal_rows);
        assert_eq!(direct.terminal_rows, 24);
        assert_eq!(
            direct.applied_revision.load(Ordering::Acquire),
            tunneled.applied_revision.load(Ordering::Acquire)
        );
        assert_eq!(direct.applied_revision.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn shared_peer_loss_keeps_each_frontend_resume_checkpoint_independent() {
        let target_device = DeviceId::from_array([0x39; DeviceId::LENGTH]);
        let first_view = ResumeViewId::from_array([0x3a; ResumeViewId::LENGTH]);
        let second_view = ResumeViewId::from_array([0x3b; ResumeViewId::LENGTH]);
        let first_attachment = AttachmentId::from_array([0x3c; AttachmentId::LENGTH]);
        let second_attachment = AttachmentId::from_array([0x3d; AttachmentId::LENGTH]);
        let first_viewport = zterm_core::terminal::TerminalSize::new(23, 79);
        let second_viewport = zterm_core::terminal::TerminalSize::new(41, 119);
        let (mut first, stale_first_peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(target_device),
            SessionId::from_array([0x3e; SessionId::LENGTH]),
            first_attachment,
        );
        let (mut second, stale_second_peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(target_device),
            SessionId::from_array([0x3f; SessionId::LENGTH]),
            second_attachment,
        );
        drop((stale_first_peer, stale_second_peer));
        let (first_stream, mut first_daemon) =
            tokio::net::UnixStream::pair().expect("create first peer-loss tunnel");
        let (second_stream, mut second_daemon) =
            tokio::net::UnixStream::pair().expect("create second peer-loss tunnel");
        first.transport = tunnel_transport(first_stream);
        second.transport = tunnel_transport(second_stream);
        first.resume_view_id = Some(first_view);
        second.resume_view_id = Some(second_view);
        first.applied_revision.store(7, Ordering::Release);
        second.applied_revision.store(13, Ordering::Release);
        first.latest_viewport = first_viewport;
        second.latest_viewport = second_viewport;

        let lost = tunnel_envelope(
            WireKind::LocalSessionTunnelClosed,
            &v2::LocalSessionTunnelClosed {
                reason: v2::LocalSessionTunnelCloseReason::TransportLost as i32,
            },
        );
        first_daemon
            .write_all(&lost)
            .await
            .expect("report first tunnel peer loss");
        second_daemon
            .write_all(&lost)
            .await
            .expect("report second tunnel peer loss");

        for (client, attachment) in [
            (&mut first, first_attachment),
            (&mut second, second_attachment),
        ] {
            let LocalAttachmentEvent::TransportState(state) = client
                .read_next_event()
                .await
                .expect("each frontend independently enters reconnect")
            else {
                panic!("peer loss did not produce a reconnect transition");
            };
            assert_eq!(
                v2::TerminalTransportState::try_from(state.state),
                Ok(v2::TerminalTransportState::Reconnecting)
            );
            assert_eq!(
                AttachmentId::try_from(state.attachment_id.expect("reconnecting attachment ID"))
                    .expect("valid reconnecting attachment ID"),
                attachment
            );
            assert!(client.reconnect_pending());
        }

        assert_eq!(first.resume_view_id, Some(first_view));
        assert_eq!(second.resume_view_id, Some(second_view));
        assert_ne!(first.resume_view_id, second.resume_view_id);
        assert_ne!(first.session_id, second.session_id);
        assert_eq!(first.applied_revision.load(Ordering::Acquire), 7);
        assert_eq!(second.applied_revision.load(Ordering::Acquire), 13);
        assert_eq!(first.latest_viewport, first_viewport);
        assert_eq!(second.latest_viewport, second_viewport);
    }

    #[tokio::test]
    async fn reconnecting_frontends_resume_independently_through_one_viewer_listener() {
        let temporary = tempfile::tempdir().expect("create multi-resume fixture directory");
        let socket = temporary.path().join("viewer.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind viewer IPC listener");
        let target_device = DeviceId::from_array([0x40; DeviceId::LENGTH]);
        let first_session = SessionId::from_array([0x41; SessionId::LENGTH]);
        let second_session = SessionId::from_array([0x42; SessionId::LENGTH]);
        let first_view = ResumeViewId::from_array([0x43; ResumeViewId::LENGTH]);
        let second_view = ResumeViewId::from_array([0x44; ResumeViewId::LENGTH]);
        let first_new_attachment = AttachmentId::from_array([0x45; AttachmentId::LENGTH]);
        let second_new_attachment = AttachmentId::from_array([0x46; AttachmentId::LENGTH]);
        let first_viewport = zterm_core::terminal::TerminalSize::new(27, 83);
        let second_viewport = zterm_core::terminal::TerminalSize::new(43, 127);

        let server = tokio::spawn(async move {
            let (mut first_stream, _) = listener
                .accept()
                .await
                .expect("accept first replacement tunnel");
            let (mut second_stream, _) = listener
                .accept()
                .await
                .expect("accept second replacement tunnel");
            tokio::join!(
                serve_replacement_tunnel_stream(
                    &mut first_stream,
                    target_device,
                    first_new_attachment,
                ),
                serve_replacement_tunnel_stream(
                    &mut second_stream,
                    target_device,
                    second_new_attachment,
                )
            )
        });

        let (mut first, first_old_peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(target_device),
            first_session,
            AttachmentId::from_array([0x47; AttachmentId::LENGTH]),
        );
        let (mut second, second_old_peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(target_device),
            second_session,
            AttachmentId::from_array([0x48; AttachmentId::LENGTH]),
        );
        drop((first_old_peer, second_old_peer));
        first.socket = socket.clone();
        second.socket = socket;
        first.resume_view_id = Some(first_view);
        second.resume_view_id = Some(second_view);
        first.applied_revision.store(19, Ordering::Release);
        second.applied_revision.store(31, Ordering::Release);
        first.latest_viewport = first_viewport;
        second.latest_viewport = second_viewport;
        first.reconnect_pending = true;
        second.reconnect_pending = true;

        let (first_result, second_result) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(first.reconnect_remote(), second.reconnect_remote())
        })
        .await
        .expect("both frontend resumes stay bounded");
        first_result.expect("first frontend resumes");
        second_result.expect("second frontend resumes");

        let requests = server.await.expect("multi-resume fixture joins");
        let mut first_assigned_attachment = None;
        let mut second_assigned_attachment = None;
        for (request, assigned_attachment) in [
            (requests.0, first_new_attachment),
            (requests.1, second_new_attachment),
        ] {
            let actual_session =
                SessionId::try_from(request.session_id.expect("resume Session ID"))
                    .expect("valid resume Session ID");
            let (expected_view, expected_revision, expected_viewport) =
                if actual_session == first_session {
                    first_assigned_attachment = Some(assigned_attachment);
                    (first_view, 19, first_viewport)
                } else {
                    assert_eq!(actual_session, second_session);
                    second_assigned_attachment = Some(assigned_attachment);
                    (second_view, 31, second_viewport)
                };
            assert_eq!(
                ResumeViewId::try_from(request.resume_view_id.expect("resume view ID"))
                    .expect("valid resume view ID"),
                expected_view
            );
            assert_eq!(request.known_revision, Some(expected_revision));
            assert_eq!(
                zterm_core::terminal::TerminalSize::try_from(
                    request.viewport.expect("resume viewport")
                )
                .expect("valid resume viewport"),
                expected_viewport
            );
        }

        for (client, expected_attachment, expected_from, expected_viewport) in [
            (
                &mut first,
                first_assigned_attachment.expect("first frontend received an attachment"),
                19,
                first_viewport,
            ),
            (
                &mut second,
                second_assigned_attachment.expect("second frontend received an attachment"),
                31,
                second_viewport,
            ),
        ] {
            let LocalAttachmentEvent::TransportState(state) = client
                .read_next_event()
                .await
                .expect("replacement epoch enters synchronization")
            else {
                panic!("replacement epoch omitted synchronization state");
            };
            assert_eq!(
                AttachmentId::try_from(state.attachment_id.expect("new attachment ID"))
                    .expect("valid new attachment ID"),
                expected_attachment
            );
            assert_eq!(
                v2::TerminalTransportState::try_from(state.state),
                Ok(v2::TerminalTransportState::Synchronizing)
            );
            let LocalAttachmentEvent::ConnectionStatus(path) = client
                .read_next_event()
                .await
                .expect("replacement epoch resets its path")
            else {
                panic!("replacement epoch omitted its path reset");
            };
            assert_eq!(
                v2::TerminalConnectionPath::try_from(path.path),
                Ok(v2::TerminalConnectionPath::Unknown)
            );
            assert_eq!(path.rtt_ms, None);
            let LocalAttachmentEvent::Delta(delta) = client
                .read_next_event()
                .await
                .expect("replacement epoch resumes from its own checkpoint")
            else {
                panic!("replacement epoch omitted its resume delta");
            };
            assert_eq!(delta.from_revision, Revision::new(expected_from));
            assert_eq!(delta.to_revision, Revision::new(expected_from + 1));
            assert_eq!(delta.size, expected_viewport);
        }

        assert_eq!(first.session_id, first_session);
        assert_eq!(second.session_id, second_session);
        assert_eq!(first.resume_view_id, Some(first_view));
        assert_eq!(second.resume_view_id, Some(second_view));
        assert_ne!(first.attachment_id, second.attachment_id);
    }

    #[tokio::test]
    async fn interrupted_outer_tunnel_frames_remain_retryable() {
        let opened = encode_message(
            WireKind::LocalSessionTunnelOpened,
            1,
            0,
            &v2::LocalSessionTunnelOpened {
                protocol_version: zterm_proto::LOCAL_SESSION_TUNNEL_VERSION,
            },
        )
        .expect("encode Opened envelope");
        let (mut client_stream, mut daemon_stream) =
            tokio::net::UnixStream::pair().expect("create interrupted-Open fixture");
        daemon_stream
            .write_all(&opened[..1])
            .await
            .expect("write partial Opened envelope");
        daemon_stream
            .shutdown()
            .await
            .expect("interrupt Opened envelope");
        let error = match read_tunnel_first(&mut client_stream).await {
            Err(error) => error,
            Ok(_) => panic!("partial Opened envelope cannot establish a tunnel"),
        };
        assert_eq!(error.kind(), DomainErrorKind::TransportUnavailable);

        let data = tunnel_envelope(
            WireKind::LocalSessionTunnelData,
            &v2::LocalSessionTunnelData { bytes: vec![0x01] },
        );
        let (client_stream, mut daemon_stream) =
            tokio::net::UnixStream::pair().expect("create interrupted-envelope fixture");
        let mut transport = tunnel_transport(client_stream);
        daemon_stream
            .write_all(&data[..1])
            .await
            .expect("write partial tunnel envelope");
        daemon_stream
            .shutdown()
            .await
            .expect("interrupt tunnel envelope");
        let error = match transport.read_item().await {
            Err(error) => error,
            Ok(_) => panic!("partial outer envelope cannot produce a Session item"),
        };
        assert_eq!(error.kind(), DomainErrorKind::TransportUnavailable);
    }

    #[tokio::test]
    async fn remote_reconnect_reuses_view_and_session_with_latest_frontend_checkpoint() {
        let temporary = tempfile::tempdir().expect("create reconnect fixture directory");
        let socket = temporary.path().join("daemon.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind reconnect fixture IPC");
        let target_device = DeviceId::from_array([0x51; DeviceId::LENGTH]);
        let session_id = SessionId::from_array([0x52; SessionId::LENGTH]);
        let old_attachment = AttachmentId::from_array([0x53; AttachmentId::LENGTH]);
        let new_attachment = AttachmentId::from_array([0x54; AttachmentId::LENGTH]);
        let resume_view_id = ResumeViewId::from_array([0x55; ResumeViewId::LENGTH]);
        let viewport = zterm_core::terminal::TerminalSize::new(31, 97);

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept reconnect tunnel");
            let first = read_first(&mut stream).await.expect("read tunnel Open");
            assert_eq!(first.frame.kind, WireKind::LocalSessionTunnelOpenRequest);
            let open: v2::LocalSessionTunnelOpenRequest = first
                .frame
                .decode_message(WireKind::LocalSessionTunnelOpenRequest)
                .expect("decode tunnel Open");
            assert_eq!(
                open.protocol_version,
                zterm_proto::LOCAL_SESSION_TUNNEL_VERSION
            );
            let opened_target: DeviceId = open
                .target_device_id
                .expect("tunnel Open has a target")
                .try_into()
                .expect("tunnel target is valid");
            assert_eq!(opened_target, target_device);

            let mut outbound = encode_message(
                WireKind::LocalSessionTunnelOpened,
                first.frame.request_id,
                0,
                &v2::LocalSessionTunnelOpened {
                    protocol_version: zterm_proto::LOCAL_SESSION_TUNNEL_VERSION,
                },
            )
            .expect("encode tunnel Opened");
            for path in [
                v2::LocalSessionTunnelPath {
                    path: v2::TerminalConnectionPath::Unknown as i32,
                    rtt_ms: None,
                },
                v2::LocalSessionTunnelPath {
                    path: v2::TerminalConnectionPath::Unknown as i32,
                    rtt_ms: None,
                },
                v2::LocalSessionTunnelPath {
                    path: v2::TerminalConnectionPath::Direct as i32,
                    rtt_ms: Some(6),
                },
                v2::LocalSessionTunnelPath {
                    path: v2::TerminalConnectionPath::Direct as i32,
                    rtt_ms: Some(6),
                },
            ] {
                outbound
                    .extend_from_slice(&tunnel_envelope(WireKind::LocalSessionTunnelPath, &path));
            }
            stream
                .write_all(&outbound)
                .await
                .expect("write tunnel Opened and initial path");

            let mut envelope_decoder = first.decoder;
            let mut queued_envelopes = first.queued;
            let data = read_frame_parts(&mut stream, &mut envelope_decoder, &mut queued_envelopes)
                .await
                .expect("read tunneled resume request");
            assert_eq!(data.kind, WireKind::LocalSessionTunnelData);
            let data: v2::LocalSessionTunnelData = data
                .decode_message(WireKind::LocalSessionTunnelData)
                .expect("decode tunneled resume bytes");
            let mut session_decoder = FrameDecoder::new();
            let mut session_frames = session_decoder
                .feed(&data.bytes)
                .expect("decode inner resume frame");
            assert_eq!(session_frames.len(), 1);
            let attach = session_frames.remove(0);
            assert_eq!(attach.kind, WireKind::TerminalAttachRequest);
            let request_id = attach.request_id;
            let attach: v2::TerminalAttachRequest = attach
                .decode_message(WireKind::TerminalAttachRequest)
                .expect("decode inner resume request");
            let attached_session: SessionId = attach
                .session_id
                .expect("resume request keeps the Session ID")
                .try_into()
                .expect("resume Session ID is valid");
            assert_eq!(attached_session, session_id);
            let attached_view: ResumeViewId = attach
                .resume_view_id
                .expect("resume request keeps the view ID")
                .try_into()
                .expect("resume view ID is valid");
            assert_eq!(attached_view, resume_view_id);
            assert_eq!(attach.known_revision, Some(11));
            let requested_viewport: zterm_core::terminal::TerminalSize = attach
                .viewport
                .expect("resume request keeps the latest viewport")
                .try_into()
                .expect("resume viewport is valid");
            assert_eq!(requested_viewport, viewport);
            let attached_target =
                resolved_target_from_wire(attach.target).expect("resume target selector is valid");
            assert_eq!(attached_target.device_id(), Some(target_device));

            let delta = zterm_core::terminal::TerminalSurfaceDelta {
                from_revision: Revision::new(11),
                to_revision: Revision::new(12),
                size: viewport,
                active_screen: zterm_core::terminal::ActiveScreen::Main,
                row_patches: Vec::new(),
                cursor: zterm_core::terminal::TerminalCursor {
                    row: 0,
                    column: 0,
                    visible: true,
                    style: Default::default(),
                },
                modes: zterm_core::terminal::TerminalModes::default(),
                scroll_metrics: Some(zterm_core::terminal::TerminalScrollMetrics {
                    epoch: Revision::ZERO,
                    revision: Revision::new(12),
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 0,
                    viewport_rows: viewport.rows,
                }),
            };
            let inner = encode_message(
                WireKind::TerminalSemanticDelta,
                request_id,
                0,
                &zterm_proto::terminal_surface_delta_message(new_attachment, delta),
            )
            .expect("encode resume delta");
            stream
                .write_all(&tunnel_envelope(
                    WireKind::LocalSessionTunnelData,
                    &v2::LocalSessionTunnelData { bytes: inner },
                ))
                .await
                .expect("write tunneled resume delta");
        });

        let (mut client, old_peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(target_device),
            session_id,
            old_attachment,
        );
        drop(old_peer);
        client.socket = socket;
        client.resume_view_id = Some(resume_view_id);
        client
            .applied_revision
            .store(11, std::sync::atomic::Ordering::Release);
        client.latest_viewport = viewport;
        client.reconnect_pending = true;
        client
            .reconnect_remote()
            .await
            .expect("frontend resumes through a replacement tunnel");
        assert_eq!(client.session_id, session_id);
        assert_eq!(client.resume_view_id, Some(resume_view_id));
        assert_eq!(client.attachment_id, new_attachment);
        assert_eq!(client.latest_viewport, viewport);
        assert_eq!(
            client
                .applied_revision
                .load(std::sync::atomic::Ordering::Acquire),
            11,
            "the resume delta is not advertised as applied before the UI installs it"
        );

        let state = client
            .read_next_event()
            .await
            .expect("replacement epoch begins synchronizing");
        let LocalAttachmentEvent::TransportState(state) = state else {
            panic!("replacement epoch must begin with transport state");
        };
        assert_eq!(
            v2::TerminalTransportState::try_from(state.state),
            Ok(v2::TerminalTransportState::Synchronizing)
        );
        assert_eq!(
            AttachmentId::try_from(state.attachment_id.expect("state attachment ID"))
                .expect("state attachment ID is valid"),
            new_attachment
        );
        let LocalAttachmentEvent::ConnectionStatus(path) = client
            .read_next_event()
            .await
            .expect("replacement epoch exposes its frontend-owned path reset")
        else {
            panic!("replacement path reset must precede its update");
        };
        assert_eq!(
            v2::TerminalConnectionPath::try_from(path.path),
            Ok(v2::TerminalConnectionPath::Unknown)
        );
        assert_eq!(path.rtt_ms, None);
        let LocalAttachmentEvent::ConnectionStatus(path) = client
            .read_next_event()
            .await
            .expect("replacement epoch exposes its collected path sample")
        else {
            panic!("replacement path sample must precede its update");
        };
        assert_eq!(
            v2::TerminalConnectionPath::try_from(path.path),
            Ok(v2::TerminalConnectionPath::Direct)
        );
        assert_eq!(path.rtt_ms, Some(6));
        let LocalAttachmentEvent::Delta(delta) = client
            .read_next_event()
            .await
            .expect("replacement epoch exposes its contiguous delta")
        else {
            panic!("replacement epoch should resume with a delta");
        };
        assert_eq!(delta.from_revision, Revision::new(11));
        assert_eq!(delta.to_revision, Revision::new(12));
        server.await.expect("reconnect fixture server joins");
    }

    #[tokio::test]
    async fn remote_reconnect_ensures_stopped_viewer_daemon_but_local_view_does_not() {
        let temporary = tempfile::tempdir().expect("create stopped-daemon fixture directory");
        let target_device = DeviceId::from_array([0x57; DeviceId::LENGTH]);
        let calls = Arc::new(AtomicU64::new(0));
        let restarter: Arc<dyn RemoteDaemonRestarter> = Arc::new(FailingRemoteDaemonRestarter {
            calls: Arc::clone(&calls),
        });

        let (mut remote, remote_peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(target_device),
            SessionId::from_array([0x58; SessionId::LENGTH]),
            AttachmentId::from_array([0x59; AttachmentId::LENGTH]),
        );
        drop(remote_peer);
        remote.socket = temporary.path().join("stopped-daemon.sock");
        remote.resume_view_id = Some(ResumeViewId::from_array([0x5a; ResumeViewId::LENGTH]));
        remote.reconnect_pending = true;
        remote.set_remote_daemon_restarter(Arc::clone(&restarter));

        let error = remote
            .reconnect_remote()
            .await
            .expect_err("a failed viewer-daemon launch is surfaced");
        assert_eq!(error.kind(), DomainErrorKind::DaemonStartTimeout);
        assert_eq!(calls.load(Ordering::Acquire), 1);

        let (mut local, _local_peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            SessionId::from_array([0x5b; SessionId::LENGTH]),
            AttachmentId::from_array([0x5c; AttachmentId::LENGTH]),
        );
        local.set_remote_daemon_restarter(restarter);
        assert!(
            local.remote_daemon_restarter.is_none(),
            "a local Session cannot survive its daemon and must not auto-restart into a new one"
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn stopped_viewer_restart_opens_a_tunnel_and_resumes_the_same_frontend_state() {
        let temporary = tempfile::tempdir().expect("create restart-resume fixture directory");
        let socket = temporary.path().join("replacement-daemon.sock");
        let target_device = DeviceId::from_array([0x5d; DeviceId::LENGTH]);
        let session_id = SessionId::from_array([0x5e; SessionId::LENGTH]);
        let old_attachment = AttachmentId::from_array([0x5f; AttachmentId::LENGTH]);
        let new_attachment = AttachmentId::from_array([0x60; AttachmentId::LENGTH]);
        let resume_view_id = ResumeViewId::from_array([0x61; ResumeViewId::LENGTH]);
        let viewport = zterm_core::terminal::TerminalSize::new(33, 101);
        let calls = Arc::new(AtomicU64::new(0));
        let restarter = Arc::new(AcceptingRemoteDaemonRestarter {
            socket: socket.clone(),
            target: target_device,
            new_attachment,
            calls: Arc::clone(&calls),
            server: StdMutex::new(None),
        });

        let (mut client, old_peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(target_device),
            session_id,
            old_attachment,
        );
        drop(old_peer);
        client.socket = socket;
        client.resume_view_id = Some(resume_view_id);
        client.applied_revision.store(17, Ordering::Release);
        client.latest_viewport = viewport;
        client.reconnect_pending = true;
        let restart_capability: Arc<dyn RemoteDaemonRestarter> = restarter.clone();
        client.set_remote_daemon_restarter(restart_capability);

        tokio::time::timeout(Duration::from_secs(2), client.reconnect_remote())
            .await
            .expect("restart and replacement tunnel stay bounded")
            .expect("replacement viewer daemon accepts the resumed Session");
        assert_eq!(calls.load(Ordering::Acquire), 1);
        assert_eq!(client.session_id, session_id);
        assert_eq!(client.resume_view_id, Some(resume_view_id));
        assert_eq!(client.attachment_id, new_attachment);
        assert_eq!(client.latest_viewport, viewport);
        assert_eq!(client.applied_revision.load(Ordering::Acquire), 17);

        let server = restarter
            .server
            .lock()
            .expect("replacement server task lock")
            .take()
            .expect("restart hook installed one replacement server");
        let attach = server
            .await
            .expect("replacement viewer-daemon fixture joins");
        assert!(!attach.create_main);
        assert!(!attach.takeover);
        assert_eq!(attach.known_revision, Some(17));
        assert_eq!(
            SessionId::try_from(attach.session_id.expect("resumed Session ID"))
                .expect("valid resumed Session ID"),
            session_id
        );
        assert_eq!(
            ResumeViewId::try_from(attach.resume_view_id.expect("resumed view ID"))
                .expect("valid resumed view ID"),
            resume_view_id
        );
        assert_eq!(
            zterm_core::terminal::TerminalSize::try_from(
                attach.viewport.expect("resumed viewport")
            )
            .expect("valid resumed viewport"),
            viewport
        );
        assert_eq!(
            resolved_target_from_wire(attach.target)
                .expect("valid resumed target")
                .device_id(),
            Some(target_device)
        );

        let LocalAttachmentEvent::TransportState(state) = client
            .read_next_event()
            .await
            .expect("replacement epoch starts synchronizing")
        else {
            panic!("replacement epoch omitted synchronization state");
        };
        assert_eq!(
            v2::TerminalTransportState::try_from(state.state),
            Ok(v2::TerminalTransportState::Synchronizing)
        );
        let LocalAttachmentEvent::ConnectionStatus(path) = client
            .read_next_event()
            .await
            .expect("replacement epoch resets its connection observation")
        else {
            panic!("replacement epoch omitted its unknown path reset");
        };
        assert_eq!(
            v2::TerminalConnectionPath::try_from(path.path),
            Ok(v2::TerminalConnectionPath::Unknown)
        );
        assert_eq!(path.rtt_ms, None);
        let LocalAttachmentEvent::Delta(delta) = client
            .read_next_event()
            .await
            .expect("replacement epoch returns a contiguous delta")
        else {
            panic!("replacement epoch omitted its contiguous delta");
        };
        assert_eq!(delta.from_revision, Revision::new(17));
        assert_eq!(delta.to_revision, Revision::new(18));
        assert_eq!(delta.size, viewport);
    }

    #[tokio::test]
    async fn remote_reconnect_resolves_pending_history_once_before_transition() {
        let (mut client, _peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::device(DeviceId::from_array([0x61; DeviceId::LENGTH])),
            SessionId::from_array([0x62; SessionId::LENGTH]),
            AttachmentId::from_array([0x63; AttachmentId::LENGTH]),
        );
        let query = TerminalHistoryWindowQuery {
            anchor: zterm_core::terminal::TerminalHistoryWindowAnchor {
                epoch: Revision::new(7),
                revision: Revision::new(9),
                max_offset_from_bottom: 20,
                viewport: zterm_core::terminal::TerminalSize::new(24, 80),
            },
            target_offset_from_bottom: 4,
            older_margin_rows: 8,
            newer_margin_rows: 0,
        };
        client.pending_history_window = Some((41, query));
        client.applied_revision.store(11, Ordering::Release);
        client.enter_reconnecting();

        assert!(client.pending_history_window.is_none());
        assert!(matches!(
            client.pending_transport_events.pop_front(),
            Some(LocalAttachmentEvent::HistoryWindow(
                TerminalSurfaceHistoryWindowResult::HistoryGap { epoch, revision }
            )) if epoch == Revision::new(7) && revision == Revision::new(11)
        ));
        assert!(matches!(
            client.pending_transport_events.pop_front(),
            Some(LocalAttachmentEvent::TransportState(state))
                if v2::TerminalTransportState::try_from(state.state)
                    == Ok(v2::TerminalTransportState::Reconnecting)
        ));
        assert!(client.pending_transport_events.is_empty());
        client.enter_reconnecting();
        assert!(
            client.pending_transport_events.is_empty(),
            "repeated failure in one reconnect epoch must not duplicate outcomes"
        );
    }

    #[tokio::test]
    async fn local_history_window_client_allows_only_one_outstanding_request() {
        let (mut client, _peer) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            SessionId::from_array([0x43; 16]),
            AttachmentId::from_array([0x44; 16]),
        );
        let query = TerminalHistoryWindowQuery {
            anchor: zterm_core::terminal::TerminalHistoryWindowAnchor {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                max_offset_from_bottom: 12,
                viewport: zterm_core::terminal::TerminalSize::new(4, 10),
            },
            target_offset_from_bottom: 1,
            older_margin_rows: 8,
            newer_margin_rows: 0,
        };
        client
            .request_history_window(query)
            .await
            .expect("first bounded window request");
        let error = client
            .request_history_window(query)
            .await
            .expect_err("second window request must await correlation");
        assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);
    }

    #[test]
    fn local_session_end_debug_never_formats_the_signal_text() {
        let event = LocalAttachmentEvent::SessionEnded(v2::TerminalSessionEnded {
            session_id: Some(SessionId::from_array([0x90; SessionId::LENGTH]).into()),
            attachment_id: Some(AttachmentId::from_array([0x91; AttachmentId::LENGTH]).into()),
            reason: v2::TerminalSessionEndReason::NaturalExit as i32,
            exit_code: 1,
            signal: "SENSITIVE_LOCAL_SIGNAL_SENTINEL".to_owned(),
        });
        let debug = format!("{event:?}");
        assert!(debug.contains("has_signal: true"));
        assert!(!debug.contains("SENSITIVE_LOCAL_SIGNAL_SENTINEL"));
    }

    #[tokio::test]
    async fn local_attachment_consumes_validated_transport_state_over_unix_duplex() {
        let session_id = SessionId::from_array([0x92; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0x93; AttachmentId::LENGTH]);
        let (mut client, mut daemon_stream) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        let event = v2::TerminalTransportStateEvent {
            attachment_id: Some(attachment_id.into()),
            state: v2::TerminalTransportState::Reconnecting as i32,
        };
        daemon_stream
            .write_all(
                &encode_message(WireKind::TerminalTransportStateEvent, 0, 0, &event)
                    .expect("bounded transport-state event"),
            )
            .await
            .expect("write transport-state event");
        match client
            .read_event(Duration::from_secs(1))
            .await
            .expect("validated transport-state event")
        {
            LocalAttachmentEvent::TransportState(actual) => assert_eq!(actual, event),
            event => panic!("unexpected local attachment event: {event:?}"),
        }

        let invalid = v2::TerminalTransportStateEvent {
            attachment_id: Some(attachment_id.into()),
            state: i32::MAX,
        };
        daemon_stream
            .write_all(
                &encode_message(WireKind::TerminalTransportStateEvent, 0, 0, &invalid)
                    .expect("bounded invalid transport-state event"),
            )
            .await
            .expect("write invalid transport-state event");
        assert_eq!(
            client
                .read_event(Duration::from_secs(1))
                .await
                .expect_err("unknown transport state is rejected")
                .kind(),
            DomainErrorKind::MalformedFrame
        );
    }

    #[tokio::test]
    async fn remote_session_stream_rejects_same_uid_transport_projections() {
        let target = ResolvedSessionTarget::device(DeviceId::from_array([0x94; DeviceId::LENGTH]));
        let session_id = SessionId::from_array([0x95; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0x96; AttachmentId::LENGTH]);
        let (mut client, mut target_stream) =
            SessionClient::terminal_driver_test_pair(target, session_id, attachment_id);
        target_stream
            .write_all(
                &encode_message(
                    WireKind::TerminalTransportStateEvent,
                    0,
                    0,
                    &v2::TerminalTransportStateEvent {
                        attachment_id: Some(attachment_id.into()),
                        state: v2::TerminalTransportState::Active as i32,
                    },
                )
                .expect("encode same-UID-only transport state"),
            )
            .await
            .expect("write invalid remote Session transport state");
        assert_eq!(
            client
                .read_event(Duration::from_secs(1))
                .await
                .expect_err("remote normal-ALPN Session rejects transport state")
                .kind(),
            DomainErrorKind::MalformedFrame
        );

        let (mut client, mut target_stream) =
            SessionClient::terminal_driver_test_pair(target, session_id, attachment_id);
        target_stream
            .write_all(
                &encode_message(
                    WireKind::TerminalConnectionStatusEvent,
                    0,
                    0,
                    &v2::TerminalConnectionStatusEvent {
                        attachment_id: Some(attachment_id.into()),
                        path: v2::TerminalConnectionPath::Direct as i32,
                        rtt_ms: Some(3),
                    },
                )
                .expect("encode same-UID-only connection status"),
            )
            .await
            .expect("write invalid remote Session connection status");
        assert_eq!(
            client
                .read_event(Duration::from_secs(1))
                .await
                .expect_err("remote normal-ALPN Session rejects connection status")
                .kind(),
            DomainErrorKind::MalformedFrame
        );
    }

    #[tokio::test]
    async fn local_attachment_clipboard_event_requires_zero_request_and_exact_identity() {
        let session_id = SessionId::from_array([0xa2; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0xa3; AttachmentId::LENGTH]);
        let (mut client, mut daemon_stream) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        let write = TerminalClipboardWrite::new("typed clipboard".to_owned())
            .expect("valid clipboard fixture");
        let event = zterm_proto::terminal_clipboard_write_message(attachment_id, write);
        daemon_stream
            .write_all(
                &encode_message(WireKind::TerminalClipboardWrite, 0, 0, &event)
                    .expect("bounded clipboard event"),
            )
            .await
            .expect("write clipboard event");
        let LocalAttachmentEvent::ClipboardWrite(write) = client
            .read_event(Duration::from_secs(1))
            .await
            .expect("validated clipboard event")
        else {
            panic!("expected typed clipboard event");
        };
        assert_eq!(write.as_str(), "typed clipboard");

        let (mut client, mut daemon_stream) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        daemon_stream
            .write_all(
                &encode_message(WireKind::TerminalClipboardWrite, 7, 0, &event)
                    .expect("nonzero request fixture"),
            )
            .await
            .expect("write nonzero request fixture");
        assert_eq!(
            client
                .read_event(Duration::from_secs(1))
                .await
                .expect_err("nonzero clipboard request id")
                .kind(),
            DomainErrorKind::MalformedFrame
        );

        let (mut client, mut daemon_stream) = SessionClient::terminal_driver_test_pair(
            ResolvedSessionTarget::local(),
            session_id,
            attachment_id,
        );
        let wrong = zterm_proto::terminal_clipboard_write_message(
            AttachmentId::from_array([0xa4; AttachmentId::LENGTH]),
            TerminalClipboardWrite::new("wrong owner".to_owned())
                .expect("valid wrong-owner fixture"),
        );
        daemon_stream
            .write_all(
                &encode_message(WireKind::TerminalClipboardWrite, 0, 0, &wrong)
                    .expect("wrong identity fixture"),
            )
            .await
            .expect("write wrong identity fixture");
        assert_eq!(
            client
                .read_event(Duration::from_secs(1))
                .await
                .expect_err("wrong clipboard attachment")
                .kind(),
            DomainErrorKind::MalformedFrame
        );
    }

    #[tokio::test]
    async fn local_attachment_discards_stale_pre_snapshot_transport_states() {
        let temporary = tempfile::tempdir().expect("temporary socket root");
        let socket_path = temporary.path().join("terminal.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).expect("local listener");
        let session_id = SessionId::from_array([0x94; SessionId::LENGTH]);
        let attachment_id = AttachmentId::from_array([0x95; AttachmentId::LENGTH]);
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept local view");
            let states = [
                v2::TerminalTransportState::Preparing,
                v2::TerminalTransportState::Synchronizing,
            ];
            for state in states {
                stream
                    .write_all(
                        &encode_message(
                            WireKind::TerminalTransportStateEvent,
                            0,
                            0,
                            &v2::TerminalTransportStateEvent {
                                attachment_id: Some(attachment_id.into()),
                                state: state as i32,
                            },
                        )
                        .expect("bounded pre-snapshot state"),
                    )
                    .await
                    .expect("write pre-snapshot state");
            }
            stream
                .write_all(
                    &encode_message(
                        WireKind::TerminalSemanticSnapshot,
                        1,
                        0,
                        &semantic_snapshot(session_id, attachment_id),
                    )
                    .expect("bounded initial snapshot"),
                )
                .await
                .expect("write initial snapshot");
            stream
                .write_all(
                    &encode_message(
                        WireKind::TerminalTransportStateEvent,
                        0,
                        0,
                        &v2::TerminalTransportStateEvent {
                            attachment_id: Some(attachment_id.into()),
                            state: v2::TerminalTransportState::Active as i32,
                        },
                    )
                    .expect("bounded post-snapshot state"),
                )
                .await
                .expect("write post-snapshot state");
        });

        let mut client = SessionClient::connect_resolved(
            &socket_path,
            ResolvedSessionTarget::local(),
            None,
            true,
            false,
            None,
        )
        .await
        .expect("connect through pre-snapshot states");
        assert_eq!(
            client
                .take_initial_snapshot()
                .expect("initial snapshot")
                .revision,
            Revision::new(1)
        );
        let event = client
            .read_event(Duration::from_secs(1))
            .await
            .expect("post-snapshot state");
        assert!(matches!(
            event,
            LocalAttachmentEvent::TransportState(state)
                if state.state == v2::TerminalTransportState::Active as i32
        ));
        server.await.expect("server task");
    }

    #[tokio::test]
    async fn create_main_post_submit_response_loss_is_outcome_unknown_for_every_target() {
        let remote = DeviceId::from_array([0x96; DeviceId::LENGTH]);
        for target in [
            ResolvedSessionTarget::local(),
            ResolvedSessionTarget::device(remote),
        ] {
            let error = run_fake_create_main(target, FakeCreateMainReply::DropAfterSubmit)
                .await
                .expect_err("post-submit response loss has an unknown create outcome");
            assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
        }
    }

    #[tokio::test]
    async fn create_main_preserves_exact_snapshot_and_correlated_error_for_every_target() {
        let remote = DeviceId::from_array([0x97; DeviceId::LENGTH]);
        for (index, target) in [
            ResolvedSessionTarget::local(),
            ResolvedSessionTarget::device(remote),
        ]
        .into_iter()
        .enumerate()
        {
            let session_id = SessionId::from_array(
                [0x98 + u8::try_from(index).expect("two target fixtures fit u8");
                    SessionId::LENGTH],
            );
            let attachment_id = AttachmentId::from_array(
                [0xa8 + u8::try_from(index).expect("two target fixtures fit u8");
                    AttachmentId::LENGTH],
            );
            let client = run_fake_create_main(
                target,
                FakeCreateMainReply::Snapshot {
                    session_id,
                    attachment_id,
                },
            )
            .await
            .expect("a complete correlated snapshot is an exact committed result");
            assert_eq!(client.session_id(), session_id);
            assert_eq!(client.attachment_id(), attachment_id);
            assert_eq!(client.is_remote(), !target.is_local());

            let error = run_fake_create_main(
                target,
                FakeCreateMainReply::ServiceError {
                    request_id: 1,
                    kind: DomainErrorKind::SessionOccupied,
                    detail: "exact occupied fixture",
                },
            )
            .await
            .expect_err("a correlated typed service error is definitive");
            assert_eq!(error.kind(), DomainErrorKind::SessionOccupied);
            assert_eq!(error.detail(), "exact occupied fixture");
        }
    }

    #[tokio::test]
    async fn create_main_prewrite_failure_stays_definitive_and_wrong_error_id_is_unknown() {
        let temporary = tempfile::tempdir().expect("temporary missing socket root");
        let remote = DeviceId::from_array([0x9a; DeviceId::LENGTH]);
        for target in [
            ResolvedSessionTarget::local(),
            ResolvedSessionTarget::device(remote),
        ] {
            let error = SessionClient::connect_resolved(
                temporary.path().join("missing.sock"),
                target,
                None,
                true,
                false,
                None,
            )
            .await
            .expect_err("connect failure occurs before any request write");
            assert_eq!(error.kind(), DomainErrorKind::DaemonStopped);

            let error = run_fake_create_main(
                target,
                FakeCreateMainReply::ServiceError {
                    request_id: 2,
                    kind: DomainErrorKind::SessionOccupied,
                    detail: "uncorrelated occupied fixture",
                },
            )
            .await
            .expect_err("an uncorrelated service error cannot prove the create outcome");
            assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn initial_attachment_deadline_covers_states_before_the_snapshot() {
        let create_main = run_fake_initial_state_stall(true).await;
        assert_eq!(
            create_main.kind(),
            DomainErrorKind::OperationOutcomeUnknown,
            "a stalled post-write create cannot be reported as definitively absent"
        );

        let existing = run_fake_initial_state_stall(false).await;
        assert_eq!(
            existing.kind(),
            DomainErrorKind::DeadlineExceeded,
            "an existing-session attach retains its bounded transport failure"
        );
    }

    #[derive(Clone, Copy)]
    enum FakeCreateMainReply {
        DropAfterSubmit,
        ServiceError {
            request_id: u64,
            kind: DomainErrorKind,
            detail: &'static str,
        },
        Snapshot {
            session_id: SessionId,
            attachment_id: AttachmentId,
        },
    }

    async fn run_fake_initial_state_stall(create_main: bool) -> DaemonError {
        let temporary = tempfile::tempdir().expect("temporary initial-state stall fixture");
        let socket = temporary.path().join("initial-state-stall.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind fake local daemon");
        let attachment_id = AttachmentId::from_array([0x9b; AttachmentId::LENGTH]);
        let (state_written_sender, state_written_receiver) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept attach request");
            let first = read_first(&mut stream)
                .await
                .expect("decode complete attach request");
            assert_eq!(first.frame.kind, WireKind::TerminalAttachRequest);
            stream
                .write_all(
                    &encode_message(
                        WireKind::TerminalTransportStateEvent,
                        0,
                        0,
                        &v2::TerminalTransportStateEvent {
                            attachment_id: Some(attachment_id.into()),
                            state: v2::TerminalTransportState::Preparing as i32,
                        },
                    )
                    .expect("bounded pre-snapshot state"),
                )
                .await
                .expect("write pre-snapshot state");
            let _ = state_written_sender.send(());
            std::future::pending::<()>().await;
        });

        let client_socket = socket.clone();
        let client = tokio::spawn(async move {
            SessionClient::connect_resolved(
                &client_socket,
                ResolvedSessionTarget::local(),
                None,
                create_main,
                false,
                None,
            )
            .await
        });
        state_written_receiver
            .await
            .expect("server crossed the complete pre-snapshot-state write barrier");
        let result = tokio::time::timeout(
            DEFAULT_DEADLINE
                .checked_mul(2)
                .expect("fixture watchdog deadline fits Duration"),
            client,
        )
        .await
        .expect("the production initial-response deadline fires before the fixture watchdog")
        .expect("initial-state client task")
        .expect_err("the initial snapshot never arrived");
        server.abort();
        let _ = server.await;
        result
    }

    async fn run_fake_create_main(
        target: ResolvedSessionTarget,
        reply: FakeCreateMainReply,
    ) -> Result<SessionClient, DaemonError> {
        let temporary = tempfile::tempdir().expect("temporary create-main fixture");
        let socket = temporary.path().join("create-main.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind fake local daemon");
        let expected_target = target.device_id();
        let (submitted_tx, submitted_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept create-main request");
            let first = tokio::time::timeout(Duration::from_secs(2), read_first(&mut stream))
                .await
                .expect("create-main request reached the write boundary")
                .expect("decode create-main request");
            let (attach, tunneled) = if let Some(expected_target) = expected_target {
                assert_eq!(first.frame.kind, WireKind::LocalSessionTunnelOpenRequest);
                let open: v2::LocalSessionTunnelOpenRequest = first
                    .frame
                    .decode_message(WireKind::LocalSessionTunnelOpenRequest)
                    .expect("decode create-main tunnel Open");
                assert_eq!(
                    DeviceId::try_from(open.target_device_id.expect("Open target"))
                        .expect("valid Open target"),
                    expected_target
                );
                stream
                    .write_all(
                        &encode_message(
                            WireKind::LocalSessionTunnelOpened,
                            first.frame.request_id,
                            0,
                            &v2::LocalSessionTunnelOpened {
                                protocol_version: zterm_proto::LOCAL_SESSION_TUNNEL_VERSION,
                            },
                        )
                        .expect("bounded tunnel Opened"),
                    )
                    .await
                    .expect("write tunnel Opened");

                let mut decoder = first.decoder;
                let mut queued = first.queued;
                let data = read_frame_parts(&mut stream, &mut decoder, &mut queued)
                    .await
                    .expect("read tunneled create-main request");
                assert_eq!(data.kind, WireKind::LocalSessionTunnelData);
                let data: v2::LocalSessionTunnelData = data
                    .decode_message(WireKind::LocalSessionTunnelData)
                    .expect("decode tunneled create-main bytes");
                let mut session_decoder = FrameDecoder::new();
                let mut frames = session_decoder
                    .feed(&data.bytes)
                    .expect("decode tunneled create-main Session frame");
                assert_eq!(frames.len(), 1);
                (frames.remove(0), true)
            } else {
                (first.frame, false)
            };
            assert_eq!(attach.kind, WireKind::TerminalAttachRequest);
            assert_eq!(
                resolved_target_from_wire(
                    attach
                        .decode_message::<v2::TerminalAttachRequest>(
                            WireKind::TerminalAttachRequest
                        )
                        .expect("attach request")
                        .target
                )
                .expect("valid target")
                .device_id(),
                expected_target
            );
            let request: v2::TerminalAttachRequest = attach
                .decode_message(WireKind::TerminalAttachRequest)
                .expect("decode create-main payload");
            assert!(request.create_main);
            submitted_tx
                .send(())
                .expect("observe request only after a complete frame was read");
            release_rx
                .await
                .expect("release fake response after submission barrier");

            match reply {
                FakeCreateMainReply::DropAfterSubmit => {}
                FakeCreateMainReply::ServiceError {
                    request_id,
                    kind,
                    detail,
                } => {
                    let response = encode_message(
                        WireKind::ServiceErrorResponse,
                        request_id,
                        0,
                        &v2::ServiceError {
                            code: kind.code().to_owned(),
                            message: detail.to_owned(),
                        },
                    )
                    .expect("bounded typed create-main error");
                    stream
                        .write_all(&tunnel_data_if_needed(response, tunneled))
                        .await
                        .expect("write typed create-main error");
                }
                FakeCreateMainReply::Snapshot {
                    session_id,
                    attachment_id,
                } => {
                    let response = encode_message(
                        WireKind::TerminalSemanticSnapshot,
                        1,
                        0,
                        &semantic_snapshot(session_id, attachment_id),
                    )
                    .expect("bounded committed create-main snapshot");
                    stream
                        .write_all(&tunnel_data_if_needed(response, tunneled))
                        .await
                        .expect("write committed create-main snapshot");
                }
            }
            let _ = stream.shutdown().await;
        });

        let client_socket = socket.clone();
        let client = tokio::spawn(async move {
            SessionClient::connect_resolved(client_socket, target, None, true, false, None).await
        });
        tokio::time::timeout(Duration::from_secs(2), submitted_rx)
            .await
            .expect("create-main submission barrier is bounded")
            .expect("fake server reports create-main submission");
        release_tx
            .send(())
            .expect("release response-loss or exact-result fixture");
        let result = tokio::time::timeout(Duration::from_secs(2), client)
            .await
            .expect("create-main client completion is bounded")
            .expect("create-main client task");
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("fake create-main server completion is bounded")
            .expect("fake create-main server task");
        result
    }

    fn tunnel_data_if_needed(bytes: Vec<u8>, tunneled: bool) -> Vec<u8> {
        if tunneled {
            tunnel_envelope(
                WireKind::LocalSessionTunnelData,
                &v2::LocalSessionTunnelData { bytes },
            )
        } else {
            bytes
        }
    }
}
