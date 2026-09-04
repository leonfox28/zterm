//! Transport-independent live session registry and attachment state machine.

#[cfg(test)]
use std::collections::VecDeque;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use iroh::SecretKey;
use tokio::sync::watch;
#[cfg(all(unix, test))]
use zterm_core::terminal::TerminalHistoryWindowAnchor;
#[cfg(unix)]
use zterm_core::terminal::{TerminalHistoryWindowQuery, TerminalSurfaceHistoryWindowResult};
use zterm_core::terminal::{
    TerminalHostEffect, TerminalSize, TerminalSurfaceDelta, TerminalSurfaceDeltaResult,
    TerminalSurfaceSnapshot,
};
use zterm_core::{
    AttachmentId, AttachmentPrincipal, ControllerLease, DaemonIncarnation, DeviceId,
    DomainErrorKind, OperationId, OperationLease, ResourceLimits, ResumeViewId, Revision,
    SessionEndReason, SessionId, SessionName, SessionSelector,
};
use zterm_platform::account::EffectiveAccount;
use zterm_platform::pty::{PtyChildState, PtyError, PtyHost, PtyPathKind, PtySession, PtySize};
use zterm_terminal::{TerminalError, TerminalModel};

use crate::error::DaemonError;
use crate::terminal_driver::{
    TerminalAttachment, TerminalDriver, TerminalDriverConfig, TerminalDriverError,
    TerminalDriverInterrupt, TerminalDriverOwnership, TerminalEffectBroker,
    spawn_background_reaper,
};

const OPERATION_RESULTS_PER_EPOCH: usize = 128;
const MAX_ACTIVE_OPERATION_EPOCHS: usize = 64;
const MAX_REPLAY_PRINCIPALS: usize = 64;
const SESSION_COMMAND_CAPACITY: usize = 16;
const SESSION_MONITOR_INTERVAL: Duration = Duration::from_millis(10);
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const COMMAND_QUEUED: u8 = 0;
const COMMAND_STARTED: u8 = 1;
const COMMAND_EXPIRED: u8 = 2;

type SpawnSession =
    dyn Fn(TerminalSize, Option<&Path>) -> Result<(PtySession, PathBuf), DaemonError> + Send + Sync;
type MutationResult = Result<SessionSummary, DaemonError>;
type FinalAttachmentUpdate = Result<Option<AttachmentUpdate>, DaemonError>;
type FinalAttachmentUpdateSlot = Arc<Mutex<Option<FinalAttachmentUpdate>>>;

/// Current user-visible state of one live session.
#[derive(Clone, Eq, PartialEq)]
pub struct SessionSummary {
    /// Stable daemon-lifetime identity.
    pub session_id: SessionId,
    /// Current unique name.
    pub name: SessionName,
    /// Latest host terminal revision.
    pub revision: Revision,
    /// Whether an attachment owns controller input.
    pub has_controller: bool,
    /// Validated working directory used to start the login shell.
    pub working_directory: PathBuf,
    /// Last accepted terminal viewport.
    pub viewport: TerminalSize,
}

impl fmt::Debug for SessionSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionSummary")
            .field("session_id", &self.session_id)
            .field("name", &self.name)
            .field("revision", &self.revision)
            .field("has_controller", &self.has_controller)
            .field("working_directory", &"[REDACTED]")
            .field(
                "working_directory_len",
                &self.working_directory.as_os_str().len(),
            )
            .field("viewport", &self.viewport)
            .finish()
    }
}

/// Latest state transition of one attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachmentLifecycle {
    /// A full snapshot is waiting for an exact acknowledgement.
    AwaitingSnapshot {
        /// Snapshot revision which must be applied atomically.
        revision: Revision,
    },
    /// This attachment owns input at the given lease generation.
    Active {
        /// Monotonic controller generation.
        generation: u64,
    },
    /// Snapshot synchronization completed and takeover may now commit.
    PreparedTakeover,
    /// Another attachment atomically took over this controller.
    LeaseLost {
        /// Generation now owned by the replacement controller.
        generation: u64,
    },
    /// The root shell and PTY have ended.
    SessionEnded(SessionEndReason),
}

/// One latest terminal update for a synchronized attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachmentUpdate {
    /// A full replacement snapshot is required.
    Snapshot(TerminalSurfaceSnapshot),
    /// One merged delta advances the current checkpoint.
    Delta(TerminalSurfaceDelta),
}

/// Result of preparing a new local or future remote attachment.
pub struct PreparedAttachment {
    /// Handle used by one duplex transport stream.
    pub attachment: Arc<SessionAttachment>,
    /// Initial full host-authoritative state.
    pub snapshot: TerminalSurfaceSnapshot,
    /// Initial merged resume update when an exact host checkpoint matched.
    #[cfg(unix)]
    pub(crate) initial_delta: Option<TerminalSurfaceDelta>,
}

/// Exact optional resume identity supplied by an authenticated remote view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RemoteResumeRequest {
    pub(crate) view_id: ResumeViewId,
    pub(crate) known_revision: Option<Revision>,
}

/// Fully validated remote-only attachment preparation arguments.
#[cfg(unix)]
pub(crate) struct RemoteAttachmentRequest {
    pub(crate) selector: Option<SessionSelector>,
    pub(crate) create_main: bool,
    pub(crate) takeover: bool,
    pub(crate) initial_viewport: Option<TerminalSize>,
    pub(crate) resume: RemoteResumeRequest,
}

/// Per-session impact of detaching one remote principal.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PrincipalDetachOutcome {
    /// Attachments removed from this session.
    pub attachments_removed: usize,
    /// Whether the removed principal owned the controller lease.
    pub controller_released: bool,
}

/// Aggregate impact of detaching one remote principal across every session.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PrincipalDetachImpact {
    /// Sessions in which at least one attachment was removed.
    pub sessions_affected: usize,
    /// Total attachments removed across all sessions.
    pub attachments_removed: usize,
    /// Number of sessions whose controller lease was released.
    pub controllers_released: usize,
}

/// Transport-independent handle to one live attachment.
pub struct SessionAttachment {
    actor: Arc<SessionActor>,
    attachment_id: AttachmentId,
    detached: Arc<AtomicBool>,
    revisions: watch::Receiver<Revision>,
    lifecycle: watch::Receiver<AttachmentLifecycle>,
    effect_broker: TerminalEffectBroker,
    effect_wakeup: watch::Receiver<()>,
    #[cfg(unix)]
    final_update: FinalAttachmentUpdateSlot,
}

impl SessionAttachment {
    /// Stable attachment identity.
    #[must_use]
    pub const fn attachment_id(&self) -> AttachmentId {
        self.attachment_id
    }

    /// Session identity shared by every attachment to the same PTY.
    #[must_use]
    pub fn session_id(&self) -> SessionId {
        self.actor.id
    }

    /// Subscribes to the latest terminal revision without a revision queue.
    pub fn revision_watch(&self) -> Result<watch::Receiver<Revision>, DaemonError> {
        Ok(self.revisions.clone())
    }

    /// Subscribes to lifecycle changes such as takeover or session end.
    pub fn lifecycle_watch(&self) -> Result<watch::Receiver<AttachmentLifecycle>, DaemonError> {
        Ok(self.lifecycle.clone())
    }

    /// Subscribes to payload-free transient host-effect wakeups.
    pub(crate) fn effect_watch(&self) -> watch::Receiver<()> {
        self.effect_wakeup.clone()
    }

    /// Takes only a transient effect bound to this exact attachment.
    pub(crate) fn take_host_effect(&self) -> Result<Option<TerminalHostEffect>, DaemonError> {
        self.effect_broker
            .take_for(self.attachment_id)
            .map_err(map_driver_error)
    }

    /// Confirms a full snapshot, or returns a newer replacement snapshot.
    pub fn snapshot_applied(
        &self,
        revision: Revision,
    ) -> Result<Option<TerminalSurfaceSnapshot>, DaemonError> {
        self.snapshot_applied_until(revision, default_deadline())
    }

    pub(crate) fn snapshot_applied_until(
        &self,
        revision: Revision,
        deadline: Instant,
    ) -> Result<Option<TerminalSurfaceSnapshot>, DaemonError> {
        self.actor
            .request(deadline, |meta, reply| SessionCommand::SnapshotApplied {
                meta,
                attachment_id: self.attachment_id,
                revision,
                reply,
            })
    }

    /// Produces one merged latest delta or a full resynchronization.
    pub fn next_update(&self) -> Result<Option<AttachmentUpdate>, DaemonError> {
        self.next_update_until(default_deadline())
    }

    pub(crate) fn next_update_until(
        &self,
        deadline: Instant,
    ) -> Result<Option<AttachmentUpdate>, DaemonError> {
        self.actor
            .request(deadline, |meta, reply| SessionCommand::NextUpdate {
                meta,
                attachment_id: self.attachment_id,
                reply,
            })
    }

    /// Produces the final drained terminal update before a terminal lifecycle event.
    #[cfg(unix)]
    pub(crate) fn final_update_until(
        &self,
        deadline: Instant,
    ) -> Result<Option<AttachmentUpdate>, DaemonError> {
        if let Some(update) = lock(&self.final_update, "attachment final update")?.take() {
            return update;
        }
        self.actor
            .request(deadline, |meta, reply| SessionCommand::FinalUpdate {
                meta,
                attachment_id: self.attachment_id,
                reply,
            })
    }

    /// Discards a client baseline and returns a fresh full snapshot.
    pub fn sync_latest(
        &self,
        known_revision: Revision,
    ) -> Result<TerminalSurfaceSnapshot, DaemonError> {
        self.sync_latest_until(known_revision, default_deadline())
    }

    pub(crate) fn sync_latest_until(
        &self,
        known_revision: Revision,
        deadline: Instant,
    ) -> Result<TerminalSurfaceSnapshot, DaemonError> {
        self.actor
            .request(deadline, |meta, reply| SessionCommand::SyncLatest {
                meta,
                attachment_id: self.attachment_id,
                known_revision,
                reply,
            })
    }

    /// Returns one stateless client-owned history window without changing any
    /// attachment scroll baseline or live checkpoint.
    #[cfg(unix)]
    pub(crate) fn history_window_until(
        &self,
        query: TerminalHistoryWindowQuery,
        deadline: Instant,
    ) -> Result<TerminalSurfaceHistoryWindowResult, DaemonError> {
        self.actor
            .request(deadline, |meta, reply| SessionCommand::HistoryWindow {
                meta,
                attachment_id: self.attachment_id,
                query,
                reply,
            })
    }

    /// Writes controller bytes only after snapshot synchronization.
    pub fn write_input(&self, bytes: &[u8]) -> Result<(), DaemonError> {
        self.write_input_until(bytes, default_deadline())
    }

    pub(crate) fn write_input_until(
        &self,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<(), DaemonError> {
        self.actor
            .request(deadline, |meta, reply| SessionCommand::WriteInput {
                meta,
                attachment_id: self.attachment_id,
                bytes: bytes.to_vec(),
                reply,
            })
    }

    /// Atomically resizes the native PTY and terminal model.
    pub fn resize(&self, size: TerminalSize) -> Result<Revision, DaemonError> {
        self.resize_until(size, default_deadline())
    }

    pub(crate) fn resize_until(
        &self,
        size: TerminalSize,
        deadline: Instant,
    ) -> Result<Revision, DaemonError> {
        self.actor
            .request(deadline, |meta, reply| SessionCommand::Resize {
                meta,
                attachment_id: self.attachment_id,
                size,
                reply,
            })
    }

    /// Releases only this attachment; the session and PTY continue running.
    pub fn detach(&self) {
        self.detached.store(true, Ordering::Release);
    }

    /// Moves an active authenticated remote controller checkpoint into the
    /// SessionActor's sole resume cell and releases its controller lease.
    #[cfg(unix)]
    pub(crate) fn detach_for_remote_resume_until(
        &self,
        deadline: Instant,
    ) -> Result<bool, DaemonError> {
        self.actor.request(deadline, |meta, reply| {
            SessionCommand::DetachForRemoteResume {
                meta,
                attachment_id: self.attachment_id,
                reply,
            }
        })
    }
}

impl Drop for SessionAttachment {
    fn drop(&mut self) {
        self.detach();
    }
}

/// The unique transport-independent service for all live sessions in one daemon.
#[derive(Clone)]
pub struct SessionService {
    inner: Arc<RegistryInner>,
    own_device_id: DeviceId,
    spawner: Arc<SpawnSession>,
    limits: ResourceLimits,
    replay: Arc<Mutex<ReplayRegistry>>,
    #[cfg(test)]
    panic_creation_after_spawn: Arc<AtomicBool>,
    #[cfg(test)]
    panic_next_creation_cleanup: Arc<AtomicBool>,
}

impl SessionService {
    /// Creates the production account-login-shell service.
    #[must_use]
    pub fn new(own_device_id: DeviceId) -> Self {
        Self::with_spawner(
            own_device_id,
            ResourceLimits::default(),
            |size, requested_cwd| {
                let account = EffectiveAccount::current().map_err(|error| {
                    DaemonError::new(DomainErrorKind::UnsupportedPlatform, error.to_string())
                })?;
                let working_directory = requested_cwd
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| account.home().to_path_buf());
                let session = PtyHost::new()
                    .spawn_current_account_login_shell(
                        PtySize::new(size.rows, size.columns),
                        Some(&working_directory),
                    )
                    .map_err(map_pty_error)?;
                Ok((session, working_directory))
            },
        )
    }

    /// Creates a service with a deterministic task-private PTY spawner.
    #[doc(hidden)]
    #[must_use]
    pub fn with_spawner<F>(own_device_id: DeviceId, limits: ResourceLimits, spawner: F) -> Self
    where
        F: Fn(TerminalSize, Option<&Path>) -> Result<(PtySession, PathBuf), DaemonError>
            + Send
            + Sync
            + 'static,
    {
        Self {
            inner: Arc::new(RegistryInner::default()),
            own_device_id,
            spawner: Arc::new(spawner),
            limits,
            replay: Arc::new(Mutex::new(ReplayRegistry::new())),
            #[cfg(test)]
            panic_creation_after_spawn: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            panic_next_creation_cleanup: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Allocates one bounded daemon-incarnation mutation lease for this principal.
    pub fn issue_operation_lease(
        &self,
        principal: AttachmentPrincipal,
    ) -> Result<OperationLease, DaemonError> {
        lock(&self.replay, "operation replay")?.issue(principal)
    }

    #[cfg(test)]
    fn panic_next_creation_after_spawn_for_test(&self) {
        self.panic_creation_after_spawn
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    fn panic_next_creation_cleanup_for_test(&self) {
        self.panic_next_creation_cleanup
            .store(true, Ordering::Release);
    }

    /// Returns the local same-UID principal for one accepted socket view.
    #[must_use]
    pub fn local_principal(&self, local_view_id: AttachmentId) -> AttachmentPrincipal {
        AttachmentPrincipal::LocalSameUid {
            own_device_id: self.own_device_id,
            local_view_id,
        }
    }

    /// Lists live sessions from short cached actor summaries.
    pub fn list(&self) -> Result<Vec<SessionSummary>, DaemonError> {
        let entries = self.inner.live_entries()?;
        let mut summaries = Vec::with_capacity(entries.len());
        for entry in entries {
            match entry.summary() {
                Ok(summary) => summaries.push(summary),
                Err(error) if error.kind() == DomainErrorKind::SessionNotFound => {}
                Err(error) => return Err(error),
            }
        }
        summaries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(summaries)
    }

    /// Creates a named session exactly once for one retained operation ID.
    pub fn create(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        name: SessionName,
        working_directory: Option<PathBuf>,
        viewport: Option<TerminalSize>,
    ) -> Result<SessionSummary, DaemonError> {
        self.create_until(
            principal,
            operation_id,
            name,
            working_directory,
            viewport,
            default_deadline(),
        )
    }

    pub(crate) fn create_until(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        name: SessionName,
        working_directory: Option<PathBuf>,
        viewport: Option<TerminalSize>,
        deadline: Instant,
    ) -> Result<SessionSummary, DaemonError> {
        let fingerprint = OperationFingerprint::Create {
            name: name.clone(),
            working_directory: working_directory.clone(),
            viewport,
        };
        self.execute_replayed(principal, operation_id, fingerprint, || {
            ensure_before_deadline(deadline)?;
            if name.is_main() {
                return Err(reserved_main());
            }
            self.create_inner(name, working_directory, viewport, false, deadline)
        })
    }

    /// Renames one live session without changing its stable identity.
    pub fn rename(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        session_id: SessionId,
        new_name: SessionName,
    ) -> Result<SessionSummary, DaemonError> {
        self.rename_until(
            principal,
            operation_id,
            session_id,
            new_name,
            default_deadline(),
        )
    }

    pub(crate) fn rename_until(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        session_id: SessionId,
        new_name: SessionName,
        deadline: Instant,
    ) -> Result<SessionSummary, DaemonError> {
        let fingerprint = OperationFingerprint::Rename {
            session_id,
            new_name: new_name.clone(),
        };
        self.execute_replayed(principal, operation_id, fingerprint, || {
            ensure_before_deadline(deadline)?;
            self.rename_inner(session_id, new_name)
        })
    }

    /// Explicitly closes only the selected session.
    pub fn close(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        session_id: SessionId,
    ) -> Result<SessionSummary, DaemonError> {
        self.close_until(principal, operation_id, session_id, default_deadline())
    }

    pub(crate) fn close_until(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        session_id: SessionId,
        deadline: Instant,
    ) -> Result<SessionSummary, DaemonError> {
        self.execute_replayed(
            principal,
            operation_id,
            OperationFingerprint::Close { session_id },
            || {
                ensure_before_deadline(deadline)?;
                self.close_inner(session_id, SessionEndReason::ExplicitClose)
            },
        )
    }

    /// Resolves a session and prepares its first full snapshot.
    pub fn prepare_attach(
        &self,
        principal: AttachmentPrincipal,
        selector: Option<SessionSelector>,
        create_main: bool,
        takeover: bool,
        initial_viewport: Option<TerminalSize>,
    ) -> Result<PreparedAttachment, DaemonError> {
        self.prepare_attach_until(
            principal,
            selector,
            create_main,
            takeover,
            initial_viewport,
            default_deadline(),
        )
    }

    pub(crate) fn prepare_attach_until(
        &self,
        principal: AttachmentPrincipal,
        selector: Option<SessionSelector>,
        create_main: bool,
        takeover: bool,
        initial_viewport: Option<TerminalSize>,
        deadline: Instant,
    ) -> Result<PreparedAttachment, DaemonError> {
        ensure_before_deadline(deadline)?;
        if let Some(size) = initial_viewport {
            validate_viewport(self.limits, size)?;
        }
        let actor = if create_main {
            if selector.is_some() {
                return Err(invalid_session(
                    "default main attach must not include a selector",
                ));
            }
            self.default_main(initial_viewport, deadline)?
        } else {
            let selector =
                selector.ok_or_else(|| invalid_session("session selector is required"))?;
            self.resolve(&selector)?
        };
        actor.request(deadline, |meta, reply| SessionCommand::PrepareAttach {
            meta,
            principal,
            takeover,
            resume: None,
            reply,
        })
    }

    /// Prepares one authenticated remote attachment with an optional exact
    /// latest-state resume baseline. Local adapters never call this path.
    #[cfg(unix)]
    pub(crate) fn prepare_remote_attach_until(
        &self,
        principal: AttachmentPrincipal,
        request: RemoteAttachmentRequest,
        deadline: Instant,
    ) -> Result<PreparedAttachment, DaemonError> {
        ensure_before_deadline(deadline)?;
        if !matches!(principal, AttachmentPrincipal::RemoteEndpoint { .. }) {
            return Err(principal_mismatch());
        }
        if let Some(size) = request.initial_viewport {
            validate_viewport(self.limits, size)?;
        }
        let actor = if request.create_main {
            if request.selector.is_some() {
                return Err(invalid_session(
                    "default main attach must not include a selector",
                ));
            }
            self.default_main(request.initial_viewport, deadline)?
        } else {
            let selector = request
                .selector
                .ok_or_else(|| invalid_session("session selector is required"))?;
            self.resolve(&selector)?
        };
        actor.request(deadline, |meta, reply| SessionCommand::PrepareAttach {
            meta,
            principal,
            takeover: request.takeover,
            resume: Some(request.resume),
            reply,
        })
    }

    /// Atomically transfers controller ownership to a synchronized pending attachment.
    pub fn takeover(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        attachment: &SessionAttachment,
    ) -> Result<SessionSummary, DaemonError> {
        self.takeover_until(principal, operation_id, attachment, default_deadline())
    }

    pub(crate) fn takeover_until(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        attachment: &SessionAttachment,
        deadline: Instant,
    ) -> Result<SessionSummary, DaemonError> {
        self.execute_takeover_replayed(
            principal,
            operation_id,
            attachment.session_id(),
            attachment.attachment_id,
            deadline,
        )
    }

    /// Commits takeover for adapters which validated an attachment ID on-stream.
    pub fn takeover_by_id(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        session_id: SessionId,
        attachment_id: AttachmentId,
    ) -> Result<SessionSummary, DaemonError> {
        self.takeover_by_id_until(
            principal,
            operation_id,
            session_id,
            attachment_id,
            default_deadline(),
        )
    }

    pub(crate) fn takeover_by_id_until(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        session_id: SessionId,
        attachment_id: AttachmentId,
        deadline: Instant,
    ) -> Result<SessionSummary, DaemonError> {
        self.execute_takeover_replayed(principal, operation_id, session_id, attachment_id, deadline)
    }

    /// Detaches every attachment owned by one remote endpoint across all live
    /// and provisional sessions without closing any session or PTY.
    pub fn detach_remote_principal(
        &self,
        device_id: DeviceId,
    ) -> Result<PrincipalDetachImpact, DaemonError> {
        self.detach_remote_principal_until(device_id, default_deadline())
    }

    pub(crate) fn detach_remote_principal_until(
        &self,
        device_id: DeviceId,
        deadline: Instant,
    ) -> Result<PrincipalDetachImpact, DaemonError> {
        #[cfg(test)]
        {
            self.detach_remote_principal_inner(device_id, deadline, None)
        }
        #[cfg(not(test))]
        {
            self.detach_remote_principal_inner(device_id, deadline)
        }
    }

    /// Counts current attachments owned by one remote endpoint without
    /// detaching them, releasing controller leases, or changing Session state.
    #[cfg(unix)]
    pub(crate) fn remote_attachment_count_until(
        &self,
        device_id: DeviceId,
        deadline: Instant,
    ) -> Result<usize, DaemonError> {
        ensure_before_deadline(deadline)?;
        let entries = self.inner.owned_entries()?;
        let mut waiters = Vec::with_capacity(entries.len());
        for entry in entries {
            match entry.actor.enqueue_command(deadline, move |meta, reply| {
                SessionCommand::CountRemoteAttachments {
                    meta,
                    device_id,
                    reply,
                }
            }) {
                Ok(waiter) => waiters.push(waiter),
                Err(error) if error.kind() == DomainErrorKind::SessionNotFound => {}
                Err(error) => return Err(error),
            }
        }

        let mut total = 0_usize;
        for waiter in waiters {
            let count = match waiter.wait(deadline) {
                Ok(count) => count,
                Err(error) if error.kind() == DomainErrorKind::SessionNotFound => continue,
                Err(error) => return Err(error),
            };
            total = total
                .checked_add(count)
                .ok_or_else(|| resource_error("remote attachment count overflowed"))?;
        }
        Ok(total)
    }

    #[cfg(test)]
    fn detach_remote_principal_until_observed(
        &self,
        device_id: DeviceId,
        deadline: Instant,
        observe: impl Fn(SessionId) -> Option<SyncSender<()>>,
    ) -> Result<PrincipalDetachImpact, DaemonError> {
        self.detach_remote_principal_inner(device_id, deadline, Some(&observe))
    }

    fn detach_remote_principal_inner(
        &self,
        device_id: DeviceId,
        deadline: Instant,
        #[cfg(test)] observe: Option<&dyn Fn(SessionId) -> Option<SyncSender<()>>>,
    ) -> Result<PrincipalDetachImpact, DaemonError> {
        ensure_before_deadline(deadline)?;
        let entries = self.inner.owned_entries()?;
        // Phase 1: admit a detach command to every owned actor. A session
        // whose actor already ended is skipped; its in-flight result would be
        // unreachable anyway. No wait happens here, so a blocked actor cannot
        // prevent another session's detach from being admitted.
        let mut waiters = Vec::with_capacity(entries.len());
        for entry in entries {
            #[cfg(test)]
            let session_id = entry.actor.id;
            match entry.actor.enqueue_command(deadline, move |meta, reply| {
                SessionCommand::DetachRemotePrincipal {
                    meta,
                    device_id,
                    #[cfg(test)]
                    processed: observe.and_then(|observer| observer(session_id)),
                    reply,
                }
            }) {
                Ok(waiter) => waiters.push(waiter),
                Err(error) if error.kind() == DomainErrorKind::SessionNotFound => {}
                Err(error) => return Err(error),
            }
        }
        // Phase 2: collect exact outcomes under the same absolute deadline.
        let mut impact = PrincipalDetachImpact::default();
        for waiter in waiters {
            let outcome = match waiter.wait(deadline) {
                Ok(outcome) => outcome,
                Err(error) if error.kind() == DomainErrorKind::SessionNotFound => continue,
                Err(error) => return Err(error),
            };
            if outcome.attachments_removed > 0 {
                impact.sessions_affected += 1;
                impact.attachments_removed += outcome.attachments_removed;
            }
            if outcome.controller_released {
                impact.controllers_released += 1;
            }
        }
        Ok(impact)
    }

    /// Explicitly closes every current session and returns their pre-stop summaries.
    pub fn shutdown(&self) -> Result<Vec<SessionSummary>, DaemonError> {
        self.shutdown_until(Instant::now() + DEFAULT_SHUTDOWN_TIMEOUT)
    }

    pub(crate) fn shutdown_until(
        &self,
        deadline: Instant,
    ) -> Result<Vec<SessionSummary>, DaemonError> {
        let cancelled_creations = self.inner.begin_shutdown()?;
        for creation in cancelled_creations {
            creation.cancel();
        }

        let mut summaries = BTreeMap::<SessionId, SessionSummary>::new();
        // Cleanup-only fallback owners can deliberately coexist with a
        // conflicting SessionId after a poisoned/corrupt registration path.
        // Lifecycle work is therefore de-duplicated by actor ownership, never
        // by the externally visible ID which may be the corrupt value.
        let mut summary_checked = BTreeSet::new();
        let mut joined = BTreeSet::new();
        let mut observed = BTreeMap::new();
        let mut cleanup_errors = Vec::new();

        loop {
            let entries = self.inner.owned_entries()?;
            // First issue close to every owner, including actors which became
            // provisional after shutdown began. Summary errors are retained
            // but never short-circuit another owner's interruption.
            for entry in &entries {
                let actor_identity = Arc::as_ptr(&entry.actor);
                observed
                    .entry(actor_identity)
                    .or_insert_with(|| entry.clone());
                if summary_checked.insert(actor_identity) {
                    match entry.summary() {
                        Ok(summary) => {
                            summaries.insert(entry.actor.id, summary);
                        }
                        Err(error) if error.kind() == DomainErrorKind::SessionNotFound => {}
                        Err(error) => cleanup_errors.push(error),
                    }
                }
                entry.actor.begin_end(SessionEndReason::DaemonStop);
            }
            // Retain every actor observed during this shutdown. Its registry
            // finalizer may compare-remove the entry immediately before its OS
            // thread returns; dropping that summary must not let a
            // successful stop skip the corresponding JoinHandle.
            for entry in observed.values() {
                let actor_identity = Arc::as_ptr(&entry.actor);
                if entry.actor.worker_done() && !joined.contains(&actor_identity) {
                    if let Err(error) = entry.actor.wait_finished_until(deadline)
                        && error.kind() != DomainErrorKind::SessionNotFound
                    {
                        cleanup_errors.push(error);
                    }
                    if let Err(error) = entry.actor.join_finished_cleanup() {
                        cleanup_errors.push(error);
                    }
                    joined.insert(actor_identity);
                }
            }
            let all_observed_joined = observed.keys().all(|identity| joined.contains(identity));
            if self.inner.owned_entries()?.is_empty()
                && self.inner.reservation_count()? == 0
                && all_observed_joined
            {
                let mut summaries = summaries.into_values().collect::<Vec<_>>();
                summaries.sort_by(|left, right| left.name.cmp(&right.name));
                if let Some(error) = cleanup_errors.into_iter().next() {
                    self.inner.resume_after_failed_shutdown()?;
                    return Err(error);
                }
                return Ok(summaries);
            }
            if Instant::now() >= deadline {
                self.inner.resume_after_failed_shutdown()?;
                let remaining = self
                    .inner
                    .owned_entries()?
                    .into_iter()
                    .map(|entry| entry.name.to_string())
                    .collect::<Vec<_>>();
                let mut remaining = remaining;
                remaining.extend(
                    observed
                        .iter()
                        .filter(|(identity, _)| !joined.contains(identity))
                        .map(|(_, entry)| entry.name.to_string()),
                );
                remaining.sort();
                remaining.dedup();
                return Err(DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    format!(
                        "session shutdown deadline elapsed; remaining sessions: {}",
                        remaining.join(", ")
                    ),
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn default_main(
        &self,
        initial_viewport: Option<TerminalSize>,
        deadline: Instant,
    ) -> Result<Arc<SessionActor>, DaemonError> {
        let name = SessionName::main();
        match self.inner.reserve_name(&name, true)? {
            NameReservation::Existing(entry) => Ok(entry.actor),
            NameReservation::Waiting(creation) => {
                creation.wait_until(deadline)?.map(|entry| entry.actor)
            }
            NameReservation::Owner(creation) => self
                .create_reserved(name, None, initial_viewport, creation, deadline)
                .map(|entry| entry.actor),
        }
    }

    fn create_inner(
        &self,
        name: SessionName,
        working_directory: Option<PathBuf>,
        viewport: Option<TerminalSize>,
        allow_main: bool,
        deadline: Instant,
    ) -> Result<SessionSummary, DaemonError> {
        if name.is_main() && !allow_main {
            return Err(reserved_main());
        }
        match self.inner.reserve_name(&name, false)? {
            NameReservation::Owner(creation) => self
                .create_reserved(name, working_directory, viewport, creation, deadline)?
                .summary(),
            NameReservation::Existing(_) | NameReservation::Waiting(_) => {
                Err(session_already_exists(&name))
            }
        }
    }

    fn create_reserved(
        &self,
        name: SessionName,
        working_directory: Option<PathBuf>,
        viewport: Option<TerminalSize>,
        creation: Arc<CreationCell>,
        deadline: Instant,
    ) -> Result<SessionEntry, DaemonError> {
        let mut owner = CreationOwner::new(
            Arc::clone(&self.inner),
            name.clone(),
            Arc::clone(&creation),
            deadline,
        );
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.create_reserved_inner(
                name,
                working_directory,
                viewport,
                &creation,
                deadline,
                &mut owner,
            )
        }))
        .unwrap_or_else(|_| Err(outcome_unknown()));
        owner.finish(result)
    }

    fn create_reserved_inner(
        &self,
        name: SessionName,
        working_directory: Option<PathBuf>,
        viewport: Option<TerminalSize>,
        creation: &Arc<CreationCell>,
        deadline: Instant,
        owner: &mut CreationOwner,
    ) -> Result<SessionEntry, DaemonError> {
        ensure_before_deadline(deadline)?;
        if creation.is_cancelled() {
            return Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                "session creation was cancelled before ownership acquisition",
            ));
        }
        let size = viewport.unwrap_or_else(|| {
            TerminalSize::new(
                self.limits.no_controller_rows,
                self.limits.no_controller_columns,
            )
        });
        validate_viewport(self.limits, size)?;
        let session_id = self
            .inner
            .reserve_creation_session(&name, creation, self.limits)?;
        owner.set_session_id(session_id);

        let model = TerminalModel::new(size, self.limits.recent_history_rows)
            .map_err(map_terminal_error)?;
        ensure_before_deadline(deadline)?;
        let (session, actual_cwd) = (self.spawner)(size, working_directory.as_deref())?;
        let mut spawned = SpawnedPtyOwner::new(session);
        #[cfg(test)]
        if self
            .panic_creation_after_spawn
            .swap(false, Ordering::AcqRel)
        {
            panic!("injected creation panic after PTY spawn");
        }
        let driver = TerminalDriver::start(spawned.take(), model, TerminalDriverConfig::default())
            .map_err(map_driver_error)?;
        let actor = SessionActor::start(
            session_id,
            size,
            driver,
            Arc::downgrade(&self.inner),
            self.limits,
        )?;
        // From this point onward the creation guard owns the started actor.
        // Registration is fallible, so the guard must be able to publish a
        // cleanup-only provisional owner before any error can unwind this
        // stack frame.
        owner.set_actor(Arc::clone(&actor), actual_cwd.clone());
        #[cfg(test)]
        if self
            .panic_next_creation_cleanup
            .swap(false, Ordering::AcqRel)
        {
            actor.panic_next_close_for_test();
        }
        let entry = SessionEntry {
            actor: Arc::clone(&actor),
            name: name.clone(),
            working_directory: actual_cwd,
            ownership: creation.ownership.clone(),
        };
        if let Err(error) = self.inner.register_provisional(session_id, entry.clone()) {
            owner.retain_provisional_for_cleanup();
            return Err(error);
        }
        actor.mark_registry_owned();
        if !self
            .inner
            .publish_name(&name, creation, session_id, &actor)?
        {
            return Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                "session publication was cancelled after PTY startup",
            ));
        }
        Ok(entry)
    }

    fn rename_inner(
        &self,
        session_id: SessionId,
        new_name: SessionName,
    ) -> Result<SessionSummary, DaemonError> {
        if new_name.is_main() {
            return Err(reserved_main());
        }
        let entry = self.inner.rename(session_id, new_name)?;
        entry.summary()
    }

    fn close_inner(
        &self,
        session_id: SessionId,
        reason: SessionEndReason,
    ) -> Result<SessionSummary, DaemonError> {
        let entry = self.inner.entry(session_id)?;
        let summary = entry.summary()?;
        entry.actor.begin_end(reason);
        entry.actor.wait_finished()?;
        entry.actor.join_finished()?;
        Ok(summary)
    }

    fn resolve(&self, selector: &SessionSelector) -> Result<Arc<SessionActor>, DaemonError> {
        self.inner.resolve(selector)
    }

    fn summary(&self, session_id: SessionId) -> Result<SessionSummary, DaemonError> {
        self.inner.entry(session_id)?.summary()
    }

    fn execute_replayed(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        fingerprint: OperationFingerprint,
        operation: impl FnOnce() -> MutationResult,
    ) -> MutationResult {
        let key = ReplayKey::new(principal, operation_id.lease.ordinal);
        let registration = {
            let mut replay = lock(&self.replay, "operation replay")?;
            replay.register(key, operation_id, fingerprint)?
        };
        match registration {
            ReplayRegistration::Join(cell) => cell.wait(),
            ReplayRegistration::Execute(cell) => {
                let mut completion = OperationCompletionGuard::new(Arc::clone(&cell));
                let result = catch_unwind(AssertUnwindSafe(operation))
                    .unwrap_or_else(|_| Err(outcome_unknown()));
                completion.complete(result.clone());
                result
            }
        }
    }

    fn execute_takeover_replayed(
        &self,
        principal: AttachmentPrincipal,
        operation_id: OperationId,
        session_id: SessionId,
        attachment_id: AttachmentId,
        deadline: Instant,
    ) -> MutationResult {
        let replay_key = ReplayKey::new(principal, operation_id.lease.ordinal);
        let operation_key = ReplayOperationKey {
            replay_key,
            operation_id,
        };
        let registration = {
            let mut replay = lock(&self.replay, "operation replay")?;
            replay.register(
                replay_key,
                operation_id,
                OperationFingerprint::Takeover { session_id },
            )?
        };
        match registration {
            ReplayRegistration::Execute(cell) => {
                let mut completion = OperationCompletionGuard::new(Arc::clone(&cell));
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let actor = self.resolve(&SessionSelector::Id(session_id))?;
                    actor.request(deadline, |meta, reply| SessionCommand::Takeover {
                        meta,
                        principal,
                        attachment_id,
                        operation_key,
                        continuation: false,
                        reply,
                    })?;
                    self.summary(session_id)
                }))
                .unwrap_or_else(|_| Err(outcome_unknown()));
                completion.complete(result.clone());
                result
            }
            ReplayRegistration::Join(cell) => {
                let retained = cell.wait()?;
                let actor = self.resolve(&SessionSelector::Id(session_id))?;
                actor.request(deadline, |meta, reply| SessionCommand::Takeover {
                    meta,
                    principal,
                    attachment_id,
                    operation_key,
                    continuation: true,
                    reply,
                })?;
                Ok(retained)
            }
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
enum OperationFingerprint {
    Create {
        name: SessionName,
        working_directory: Option<PathBuf>,
        viewport: Option<TerminalSize>,
    },
    Rename {
        session_id: SessionId,
        new_name: SessionName,
    },
    Close {
        session_id: SessionId,
    },
    Takeover {
        session_id: SessionId,
    },
}

impl fmt::Debug for OperationFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Create {
                name,
                working_directory,
                viewport,
            } => formatter
                .debug_struct("Create")
                .field("name", name)
                .field("working_directory", &"[REDACTED]")
                .field("working_directory_present", &working_directory.is_some())
                .field("viewport", viewport)
                .finish(),
            Self::Rename {
                session_id,
                new_name,
            } => formatter
                .debug_struct("Rename")
                .field("session_id", session_id)
                .field("new_name", new_name)
                .finish(),
            Self::Close { session_id } => formatter
                .debug_struct("Close")
                .field("session_id", session_id)
                .finish(),
            Self::Takeover { session_id } => formatter
                .debug_struct("Takeover")
                .field("session_id", session_id)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReplayOperationKey {
    replay_key: ReplayKey,
    operation_id: OperationId,
}

#[derive(Clone)]
struct SessionEntry {
    actor: Arc<SessionActor>,
    name: SessionName,
    working_directory: PathBuf,
    ownership: OwnershipToken,
}

impl SessionEntry {
    fn summary(&self) -> Result<SessionSummary, DaemonError> {
        let runtime = self.actor.runtime_summary()?;
        Ok(SessionSummary {
            session_id: self.actor.id,
            name: self.name.clone(),
            revision: self.actor.latest_revision(),
            has_controller: runtime.has_controller,
            working_directory: self.working_directory.clone(),
            viewport: runtime.viewport,
        })
    }
}

/// Unforgeable in-process ownership identity shared by a creation name slot,
/// its session-count reservation, and the eventual provisional/live actor entry.
/// Cleanup always compares this identity before removing any reservation.
#[derive(Clone)]
struct OwnershipToken(Arc<()>);

impl OwnershipToken {
    fn new() -> Self {
        Self(Arc::new(()))
    }

    fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Default)]
struct RegistryInner {
    state: Mutex<RegistryState>,
    reservations: Mutex<ReservationState>,
    #[cfg(test)]
    candidate_ids: Mutex<VecDeque<SessionId>>,
    #[cfg(test)]
    fail_provisional_registration: AtomicBool,
}

impl RegistryInner {
    fn reserve_name(
        &self,
        name: &SessionName,
        wait_for_starting: bool,
    ) -> Result<NameReservation, DaemonError> {
        let mut state = lock(&self.state, "session registry")?;
        if !state.accepting {
            return Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                "session registry is shutting down",
            ));
        }
        match state.by_name.get(name) {
            Some(NameSlot::Live { session_id, .. }) => {
                let entry = state
                    .by_id
                    .get(session_id)
                    .cloned()
                    .ok_or_else(session_not_found)?;
                Ok(NameReservation::Existing(entry))
            }
            Some(NameSlot::Starting { creation, .. }) if wait_for_starting => {
                Ok(NameReservation::Waiting(Arc::clone(creation)))
            }
            Some(NameSlot::Starting { .. }) => Err(session_already_exists(name)),
            None => {
                let creation = Arc::new(CreationCell::default());
                state.by_name.insert(
                    name.clone(),
                    NameSlot::Starting {
                        creation: Arc::clone(&creation),
                        session_id: None,
                    },
                );
                Ok(NameReservation::Owner(creation))
            }
        }
    }

    fn publish_name(
        &self,
        name: &SessionName,
        creation: &Arc<CreationCell>,
        session_id: SessionId,
        actor: &Arc<SessionActor>,
    ) -> Result<bool, DaemonError> {
        let mut state = lock(&self.state, "session registry")?;
        let owns_reservation = state.accepting
            && !creation.is_cancelled()
            && state.by_name.get(name).is_some_and(|slot| {
                matches!(
                    slot,
                    NameSlot::Starting {
                        creation: current,
                        session_id: Some(current_id),
                    } if Arc::ptr_eq(current, creation) && *current_id == session_id
                )
            });
        if !owns_reservation {
            return Ok(false);
        }
        let owns_provisional = state
            .provisional
            .get(&session_id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.actor, actor));
        if !owns_provisional {
            return Ok(false);
        }
        let Some(entry) = state.provisional.remove(&session_id) else {
            return Ok(false);
        };
        state.by_name.insert(
            name.clone(),
            NameSlot::Live {
                session_id,
                ownership: entry.ownership.clone(),
            },
        );
        state.by_id.insert(session_id, entry);
        Ok(true)
    }

    fn register_provisional(
        &self,
        session_id: SessionId,
        entry: SessionEntry,
    ) -> Result<(), DaemonError> {
        #[cfg(test)]
        if self
            .fail_provisional_registration
            .swap(false, Ordering::AcqRel)
        {
            return Err(synchronization_error("injected provisional registration"));
        }
        let mut state = lock(&self.state, "session registry")?;
        if state.by_id.contains_key(&session_id)
            || state.provisional.contains_key(&session_id)
            || state
                .cleanup_only
                .iter()
                .any(|entry| entry.actor.id == session_id)
        {
            return Err(resource_error("duplicate provisional session identity"));
        }
        let owns_name = state.by_name.get(&entry.name).is_some_and(|slot| {
            matches!(
                slot,
                NameSlot::Starting {
                    creation,
                    session_id: Some(current_id),
                } if *current_id == session_id && creation.ownership.ptr_eq(&entry.ownership)
            )
        });
        if !owns_name {
            return Err(resource_error(
                "provisional actor does not own its starting name slot",
            ));
        }
        state.provisional.insert(session_id, entry);
        Ok(())
    }

    /// Cleanup fallback for an actor which started before ordinary provisional
    /// registration reported an error (for example, a poisoned normal lock).
    /// Poison is recovered here and the exact name/resource token is checked;
    /// an unrelated owner is never overwritten.
    fn retain_provisional_for_cleanup(&self, session_id: SessionId, entry: SessionEntry) {
        let mut state = cleanup_lock(&self.state);
        if state
            .provisional
            .get(&session_id)
            .is_some_and(|current| Arc::ptr_eq(&current.actor, &entry.actor))
            || state
                .by_id
                .get(&session_id)
                .is_some_and(|current| Arc::ptr_eq(&current.actor, &entry.actor))
            || state
                .cleanup_only
                .iter()
                .any(|current| Arc::ptr_eq(&current.actor, &entry.actor))
        {
            return;
        }
        let owns_name = state.by_name.get(&entry.name).is_some_and(|slot| {
            matches!(
                slot,
                NameSlot::Starting {
                    creation,
                    session_id: Some(current_id),
                } if *current_id == session_id && creation.ownership.ptr_eq(&entry.ownership)
            )
        });
        let reservations = cleanup_lock(&self.reservations);
        let owns_reservation = reservations
            .reservations
            .get(&session_id)
            .is_some_and(|ownership| ownership.ptr_eq(&entry.ownership));
        drop(reservations);
        if owns_name
            && owns_reservation
            && !state.by_id.contains_key(&session_id)
            && !state.provisional.contains_key(&session_id)
        {
            state.provisional.insert(session_id, entry);
        } else {
            // An impossible/corrupt ID conflict still must not hide a started
            // actor. This cleanup-only owner is included in shutdown and is
            // removed by actor identity, without touching unrelated tokens.
            state.cleanup_only.push(entry);
        }
    }

    fn rename(
        &self,
        session_id: SessionId,
        new_name: SessionName,
    ) -> Result<SessionEntry, DaemonError> {
        let mut state = lock(&self.state, "session registry")?;
        if !state.accepting {
            return Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                "session registry is shutting down",
            ));
        }
        let current = state
            .by_id
            .get(&session_id)
            .cloned()
            .ok_or_else(session_not_found)?;
        if current.name.is_main() {
            return Err(reserved_main());
        }
        if state.by_name.contains_key(&new_name) {
            return Err(session_already_exists(&new_name));
        }
        let owns_old_name = state.by_name.get(&current.name).is_some_and(|slot| {
            matches!(
                slot,
                NameSlot::Live {
                    session_id: current_id,
                    ownership,
                } if *current_id == session_id && ownership.ptr_eq(&current.ownership)
            )
        });
        if !owns_old_name {
            return Err(synchronization_error("session name ownership"));
        }
        state.by_name.remove(&current.name);
        state.by_name.insert(
            new_name.clone(),
            NameSlot::Live {
                session_id,
                ownership: current.ownership.clone(),
            },
        );
        let updated = SessionEntry {
            name: new_name,
            ..current
        };
        state.by_id.insert(session_id, updated.clone());
        Ok(updated)
    }

    fn resolve(&self, selector: &SessionSelector) -> Result<Arc<SessionActor>, DaemonError> {
        let state = lock(&self.state, "session registry")?;
        let entry = match selector {
            SessionSelector::Id(session_id) => state.by_id.get(session_id),
            SessionSelector::Name(name) => state.by_name.get(name).and_then(|slot| match slot {
                NameSlot::Live { session_id, .. } => state.by_id.get(session_id),
                NameSlot::Starting { .. } => None,
            }),
        };
        entry
            .map(|entry| Arc::clone(&entry.actor))
            .ok_or_else(session_not_found)
    }

    fn entry(&self, session_id: SessionId) -> Result<SessionEntry, DaemonError> {
        lock(&self.state, "session registry")?
            .by_id
            .get(&session_id)
            .cloned()
            .ok_or_else(session_not_found)
    }

    fn live_entries(&self) -> Result<Vec<SessionEntry>, DaemonError> {
        Ok(lock(&self.state, "session registry")?
            .by_id
            .values()
            .cloned()
            .collect())
    }

    fn reserve_creation_session(
        &self,
        name: &SessionName,
        creation: &Arc<CreationCell>,
        limits: ResourceLimits,
    ) -> Result<SessionId, DaemonError> {
        // State-before-reservations is also used by completion. Holding the state
        // lock makes shutdown cancellation and identity/session-count reservation
        // one atomic ownership boundary. No path may acquire these locks in
        // the opposite order.
        let mut state = lock(&self.state, "session registry")?;
        let owns_reservation = state.accepting
            && !creation.is_cancelled()
            && state.by_name.get(name).is_some_and(|slot| {
                matches!(
                    slot,
                    NameSlot::Starting {
                        creation: current,
                        session_id: None,
                    } if Arc::ptr_eq(current, creation)
                )
            });
        if !owns_reservation {
            return Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                "session creation reservation was cancelled",
            ));
        }
        let mut reservations = lock(&self.reservations, "session reservations")?;
        if reservations.reservations.len() >= limits.max_live_sessions {
            return Err(resource_error("live session limit reached"));
        }
        let session_id = (0..16)
            .map(|_| self.next_session_id_candidate())
            .find(|candidate| {
                !state.by_id.contains_key(candidate)
                    && !state.provisional.contains_key(candidate)
                    && !state
                        .cleanup_only
                        .iter()
                        .any(|entry| entry.actor.id == *candidate)
                    && !reservations.reservations.contains_key(candidate)
            })
            .ok_or_else(|| resource_error("unable to allocate a unique session identity"))?;
        let replaced = reservations
            .reservations
            .insert(session_id, creation.ownership.clone());
        debug_assert!(
            replaced.is_none(),
            "atomic identity reservation replaced an owner"
        );
        let Some(NameSlot::Starting {
            creation: current,
            session_id: current_id,
        }) = state.by_name.get_mut(name)
        else {
            unreachable!("verified starting name slot disappeared while locked")
        };
        debug_assert!(Arc::ptr_eq(current, creation));
        *current_id = Some(session_id);
        Ok(session_id)
    }

    fn next_session_id_candidate(&self) -> SessionId {
        #[cfg(test)]
        if let Some(candidate) = cleanup_lock(&self.candidate_ids).pop_front() {
            return candidate;
        }
        SessionId::from_array(random_16())
    }

    #[cfg(test)]
    fn inject_session_id_candidates(&self, candidates: impl IntoIterator<Item = SessionId>) {
        cleanup_lock(&self.candidate_ids).extend(candidates);
    }

    #[cfg(test)]
    fn fail_next_provisional_registration(&self) {
        self.fail_provisional_registration
            .store(true, Ordering::Release);
    }

    /// Releases a creation which never acquired a registry-visible actor.
    /// Both the name and session-count reservation are retained unless this exact creation
    /// token still owns them.
    fn release_creation(
        &self,
        name: &SessionName,
        creation: &Arc<CreationCell>,
        session_id: Option<SessionId>,
    ) {
        let mut state = cleanup_lock(&self.state);
        let owns_name = state.by_name.get(name).is_some_and(|slot| {
            matches!(
                slot,
                NameSlot::Starting {
                    creation: current,
                    session_id: current_id,
                } if Arc::ptr_eq(current, creation) && *current_id == session_id
            )
        });
        if let Some(session_id) = session_id {
            let mut reservations = cleanup_lock(&self.reservations);
            let owns_reservation = reservations
                .reservations
                .get(&session_id)
                .is_some_and(|ownership| ownership.ptr_eq(&creation.ownership));
            if owns_reservation {
                reservations.reservations.remove(&session_id);
            }
        }
        if owns_name {
            state.by_name.remove(name);
        }
    }

    fn reservation_count(&self) -> Result<usize, DaemonError> {
        Ok(cleanup_lock(&self.reservations).reservations.len())
    }

    fn complete(&self, session_id: SessionId, actor: &Arc<SessionActor>) {
        let mut state = cleanup_lock(&self.state);
        let live_matches = state
            .by_id
            .get(&session_id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.actor, actor));
        let provisional_matches = state
            .provisional
            .get(&session_id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.actor, actor));
        let cleanup_position = state
            .cleanup_only
            .iter()
            .position(|entry| Arc::ptr_eq(&entry.actor, actor));
        let entry = if live_matches {
            state.by_id.get(&session_id).cloned()
        } else if provisional_matches {
            state.provisional.get(&session_id).cloned()
        } else {
            cleanup_position.map(|position| state.cleanup_only[position].clone())
        };
        let Some(entry) = entry else {
            return;
        };
        let owns_name = state
            .by_name
            .get(&entry.name)
            .is_some_and(|slot| match slot {
                NameSlot::Live {
                    session_id: current,
                    ownership,
                } => *current == session_id && ownership.ptr_eq(&entry.ownership),
                NameSlot::Starting {
                    creation,
                    session_id: Some(current),
                } => *current == session_id && creation.ownership.ptr_eq(&entry.ownership),
                NameSlot::Starting {
                    session_id: None, ..
                } => false,
            });
        let mut reservations = cleanup_lock(&self.reservations);
        let owns_reservation = reservations
            .reservations
            .get(&session_id)
            .is_some_and(|ownership| ownership.ptr_eq(&entry.ownership));
        if live_matches {
            state.by_id.remove(&session_id);
        } else if provisional_matches {
            state.provisional.remove(&session_id);
        } else if let Some(position) = cleanup_position {
            state.cleanup_only.remove(position);
        }
        if owns_name {
            state.by_name.remove(&entry.name);
        }
        if owns_reservation {
            reservations.reservations.remove(&session_id);
        }
    }

    fn begin_shutdown(&self) -> Result<Vec<Arc<CreationCell>>, DaemonError> {
        let mut state = cleanup_lock(&self.state);
        state.accepting = false;
        let creations = state
            .by_name
            .values()
            .filter_map(|slot| match slot {
                NameSlot::Starting { creation, .. } => Some(Arc::clone(creation)),
                NameSlot::Live { .. } => None,
            })
            .collect::<Vec<_>>();
        Ok(creations)
    }

    fn owned_entries(&self) -> Result<Vec<SessionEntry>, DaemonError> {
        let state = cleanup_lock(&self.state);
        let mut entries = state.by_id.values().cloned().collect::<Vec<_>>();
        entries.extend(state.provisional.values().cloned());
        entries.extend(state.cleanup_only.iter().cloned());
        Ok(entries)
    }

    fn resume_after_failed_shutdown(&self) -> Result<(), DaemonError> {
        cleanup_lock(&self.state).accepting = true;
        Ok(())
    }
}

struct RegistryState {
    by_id: BTreeMap<SessionId, SessionEntry>,
    provisional: BTreeMap<SessionId, SessionEntry>,
    cleanup_only: Vec<SessionEntry>,
    by_name: BTreeMap<SessionName, NameSlot>,
    accepting: bool,
}

impl Default for RegistryState {
    fn default() -> Self {
        Self {
            by_id: BTreeMap::new(),
            provisional: BTreeMap::new(),
            cleanup_only: Vec::new(),
            by_name: BTreeMap::new(),
            accepting: true,
        }
    }
}

enum NameSlot {
    Starting {
        creation: Arc<CreationCell>,
        session_id: Option<SessionId>,
    },
    Live {
        session_id: SessionId,
        ownership: OwnershipToken,
    },
}

enum NameReservation {
    Owner(Arc<CreationCell>),
    Waiting(Arc<CreationCell>),
    Existing(SessionEntry),
}

struct CreationCell {
    result: Mutex<Option<Result<SessionEntry, DaemonError>>>,
    changed: Condvar,
    cancelled: AtomicBool,
    ownership: OwnershipToken,
}

impl Default for CreationCell {
    fn default() -> Self {
        Self {
            result: Mutex::new(None),
            changed: Condvar::new(),
            cancelled: AtomicBool::new(false),
            ownership: OwnershipToken::new(),
        }
    }
}

impl CreationCell {
    fn complete(&self, result: Result<SessionEntry, DaemonError>) {
        let mut current = cleanup_lock(&self.result);
        if current.is_none() {
            *current = Some(result);
            self.changed.notify_all();
        }
    }

    fn wait_until(
        &self,
        deadline: Instant,
    ) -> Result<Result<SessionEntry, DaemonError>, DaemonError> {
        let mut result = lock(&self.result, "session creation")?;
        loop {
            if let Some(result) = result.clone() {
                return Ok(result);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(deadline_error(
                    "session creation did not finish before its deadline",
                ));
            }
            let (next, timeout) = self
                .changed
                .wait_timeout(result, deadline.saturating_duration_since(now))
                .map_err(|_| synchronization_error("session creation"))?;
            result = next;
            if timeout.timed_out() && result.is_none() {
                return Err(deadline_error(
                    "session creation did not finish before its deadline",
                ));
            }
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.complete(Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            "session creation was cancelled by daemon shutdown",
        )));
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

struct CreationOwner {
    registry: Arc<RegistryInner>,
    name: SessionName,
    creation: Arc<CreationCell>,
    session_id: Option<SessionId>,
    actor: Option<SessionEntry>,
    cleanup_deadline: Instant,
    finished: bool,
}

impl CreationOwner {
    fn new(
        registry: Arc<RegistryInner>,
        name: SessionName,
        creation: Arc<CreationCell>,
        cleanup_deadline: Instant,
    ) -> Self {
        Self {
            registry,
            name,
            creation,
            session_id: None,
            actor: None,
            cleanup_deadline,
            finished: false,
        }
    }

    fn set_session_id(&mut self, session_id: SessionId) {
        self.session_id = Some(session_id);
    }

    fn set_actor(&mut self, actor: Arc<SessionActor>, working_directory: PathBuf) {
        self.actor = Some(SessionEntry {
            actor,
            name: self.name.clone(),
            working_directory,
            ownership: self.creation.ownership.clone(),
        });
    }

    fn retain_provisional_for_cleanup(&self) {
        let (Some(session_id), Some(entry)) = (self.session_id, self.actor.as_ref()) else {
            return;
        };
        self.registry
            .retain_provisional_for_cleanup(session_id, entry.clone());
        entry.actor.mark_registry_owned();
    }

    fn finish(
        &mut self,
        mut result: Result<SessionEntry, DaemonError>,
    ) -> Result<SessionEntry, DaemonError> {
        if result.is_err()
            && let Err(cleanup) = self.cleanup_owned()
        {
            result = Err(cleanup);
        }
        self.creation.complete(result.clone());
        self.finished = true;
        result
    }

    fn cleanup_owned(&mut self) -> Result<(), DaemonError> {
        if let Some(entry) = &self.actor {
            // Registration failure must not turn this into an invisible child.
            // The cleanup registry holds the actor before this bounded wait;
            // eventual worker completion compare-removes it and the name slot.
            self.retain_provisional_for_cleanup();
            entry.actor.begin_end(SessionEndReason::DriverFailure);
            entry.actor.wait_finished_until(self.cleanup_deadline)?;
            entry.actor.join_finished()?;
        } else {
            self.registry
                .release_creation(&self.name, &self.creation, self.session_id.take());
        }
        Ok(())
    }
}

impl Drop for CreationOwner {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let _ = self.cleanup_owned();
        self.creation.complete(Err(outcome_unknown()));
    }
}

struct SpawnedPtyOwner(Option<PtySession>);

impl SpawnedPtyOwner {
    fn new(session: PtySession) -> Self {
        Self(Some(session))
    }

    fn take(&mut self) -> PtySession {
        self.0.take().expect("spawned PTY owner contains a session")
    }
}

impl Drop for SpawnedPtyOwner {
    fn drop(&mut self) {
        if let Some(mut session) = self.0.take() {
            let _ = session.close_explicitly();
        }
    }
}

#[derive(Default)]
struct ReservationState {
    reservations: BTreeMap<SessionId, OwnershipToken>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReplayPrincipal {
    device_id: DeviceId,
    authorization_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReplayKey {
    principal: ReplayPrincipal,
    lease_ordinal: u64,
}

impl ReplayKey {
    fn new(principal: AttachmentPrincipal, lease_ordinal: u64) -> Self {
        Self {
            principal: ReplayPrincipal::from_attachment(principal),
            lease_ordinal,
        }
    }
}

impl ReplayPrincipal {
    fn from_attachment(principal: AttachmentPrincipal) -> Self {
        match principal {
            AttachmentPrincipal::LocalSameUid { own_device_id, .. } => ReplayPrincipal {
                device_id: own_device_id,
                authorization_generation: 0,
            },
            AttachmentPrincipal::RemoteEndpoint {
                device_id,
                auth_generation,
            } => ReplayPrincipal {
                device_id,
                authorization_generation: auth_generation,
            },
        }
    }
}

struct ReplayRegistry {
    incarnation: DaemonIncarnation,
    epochs: BTreeMap<ReplayKey, ReplayEpochEntry>,
    retired_through: BTreeMap<ReplayPrincipal, u64>,
    issued_through: BTreeMap<ReplayPrincipal, u64>,
    access_clock: u64,
}

struct ReplayEpochEntry {
    epoch: Arc<ReplayEpoch>,
    last_access: u64,
}

impl ReplayRegistry {
    fn new() -> Self {
        Self {
            incarnation: DaemonIncarnation::from_array(random_16()),
            epochs: BTreeMap::new(),
            retired_through: BTreeMap::new(),
            issued_through: BTreeMap::new(),
            access_clock: 0,
        }
    }

    fn issue(
        &mut self,
        attachment_principal: AttachmentPrincipal,
    ) -> Result<OperationLease, DaemonError> {
        let principal = ReplayPrincipal::from_attachment(attachment_principal);
        if !self.issued_through.contains_key(&principal)
            && self.issued_through.len() >= MAX_REPLAY_PRINCIPALS
        {
            return Err(resource_error("too many operation replay principals"));
        }
        let ordinal = self
            .issued_through
            .get(&principal)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| resource_error("operation lease ordinal exhausted"))?;
        while self.epochs.len() >= MAX_ACTIVE_OPERATION_EPOCHS {
            self.retire_one()?;
        }
        let last_access = self.next_access();
        self.epochs.insert(
            ReplayKey {
                principal,
                lease_ordinal: ordinal,
            },
            ReplayEpochEntry {
                epoch: Arc::new(ReplayEpoch::default()),
                last_access,
            },
        );
        self.issued_through.insert(principal, ordinal);
        Ok(OperationLease {
            daemon_incarnation: self.incarnation,
            ordinal,
        })
    }

    fn register(
        &mut self,
        key: ReplayKey,
        operation_id: OperationId,
        fingerprint: OperationFingerprint,
    ) -> Result<ReplayRegistration, DaemonError> {
        // Incarnation is deliberately first: an old-daemon token must not
        // influence this daemon's ordinal floor or allocation state.
        if operation_id.lease.daemon_incarnation != self.incarnation {
            return Err(outcome_unknown());
        }
        if operation_id.sequence == 0 || operation_id.lease.ordinal != key.lease_ordinal {
            return Err(outcome_unknown());
        }
        if self
            .retired_through
            .get(&key.principal)
            .is_some_and(|floor| key.lease_ordinal <= *floor)
        {
            return Err(outcome_unknown());
        }
        if self
            .issued_through
            .get(&key.principal)
            .is_none_or(|issued| key.lease_ordinal > *issued)
        {
            return Err(outcome_unknown());
        }
        let access = self.next_access();
        if let Some(entry) = self.epochs.get_mut(&key) {
            entry.last_access = access;
            return entry.epoch.register(operation_id.sequence, fingerprint);
        }
        // Every executable epoch is daemon-issued and registry-resident. A
        // missing issued epoch was retired; it must never be recreated.
        Err(outcome_unknown())
    }

    fn next_access(&mut self) -> u64 {
        if self.access_clock == u64::MAX {
            let mut order = self
                .epochs
                .iter()
                .map(|(key, entry)| (*key, entry.last_access))
                .collect::<Vec<_>>();
            order.sort_by_key(|(_, access)| *access);
            for (index, (key, _)) in order.into_iter().enumerate() {
                if let Some(entry) = self.epochs.get_mut(&key) {
                    entry.last_access = u64::try_from(index).unwrap_or(u64::MAX - 1);
                }
            }
            self.access_clock = u64::try_from(self.epochs.len()).unwrap_or(u64::MAX - 1);
        }
        self.access_clock += 1;
        self.access_clock
    }

    fn retire_one(&mut self) -> Result<(), DaemonError> {
        let mut candidate = None;
        for (key, entry) in &self.epochs {
            let mut prefix_complete = true;
            for (active, active_entry) in &self.epochs {
                if active.principal == key.principal
                    && active.lease_ordinal <= key.lease_ordinal
                    && !active_entry.epoch.is_fully_complete()?
                {
                    prefix_complete = false;
                    break;
                }
            }
            if prefix_complete
                && candidate.is_none_or(|(_, last_access)| entry.last_access < last_access)
            {
                candidate = Some((*key, entry.last_access));
            }
        }
        let key = candidate.map(|(key, _)| key).ok_or_else(|| {
            resource_error("operation replay registry is full of in-flight epochs")
        })?;
        let retirement_floor = self
            .retired_through
            .get(&key.principal)
            .copied()
            .unwrap_or(0)
            .max(key.lease_ordinal);
        self.retired_through.insert(key.principal, retirement_floor);
        self.epochs.retain(|candidate, _| {
            candidate.principal != key.principal || candidate.lease_ordinal > retirement_floor
        });
        Ok(())
    }
}

#[derive(Default)]
struct ReplayEpoch {
    state: Mutex<ReplayEpochState>,
}

#[derive(Default)]
struct ReplayEpochState {
    low_water: u64,
    operations: BTreeMap<u64, Arc<OperationCell>>,
}

impl ReplayEpoch {
    fn register(
        &self,
        sequence: u64,
        fingerprint: OperationFingerprint,
    ) -> Result<ReplayRegistration, DaemonError> {
        let mut state = lock(&self.state, "operation epoch")?;
        if let Some(cell) = state.operations.get(&sequence) {
            if cell.fingerprint != fingerprint {
                return Err(outcome_unknown());
            }
            return Ok(ReplayRegistration::Join(Arc::clone(cell)));
        }
        if sequence == 0 || sequence < state.low_water {
            return Err(outcome_unknown());
        }
        while state.operations.len() >= OPERATION_RESULTS_PER_EPOCH {
            let Some((&oldest, cell)) = state.operations.first_key_value() else {
                break;
            };
            if !cell.is_complete()? {
                return Err(resource_error(
                    "operation replay window is full of in-flight operations",
                ));
            }
            state.operations.remove(&oldest);
            state.low_water = state.low_water.max(oldest.saturating_add(1));
        }
        let cell = Arc::new(OperationCell::new(fingerprint));
        state.operations.insert(sequence, Arc::clone(&cell));
        Ok(ReplayRegistration::Execute(cell))
    }

    fn is_fully_complete(&self) -> Result<bool, DaemonError> {
        let state = lock(&self.state, "operation epoch")?;
        for cell in state.operations.values() {
            if !cell.is_complete()? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

enum ReplayRegistration {
    Execute(Arc<OperationCell>),
    Join(Arc<OperationCell>),
}

struct OperationCell {
    fingerprint: OperationFingerprint,
    result: Mutex<Option<MutationResult>>,
    changed: Condvar,
}

impl OperationCell {
    fn new(fingerprint: OperationFingerprint) -> Self {
        Self {
            fingerprint,
            result: Mutex::new(None),
            changed: Condvar::new(),
        }
    }

    fn complete(&self, result: MutationResult) {
        let mut current = cleanup_lock(&self.result);
        if current.is_none() {
            *current = Some(result);
            self.changed.notify_all();
        }
    }

    fn wait(&self) -> MutationResult {
        let mut result = lock(&self.result, "operation result")?;
        loop {
            if let Some(result) = result.clone() {
                return result;
            }
            result = self
                .changed
                .wait(result)
                .map_err(|_| synchronization_error("operation result"))?;
        }
    }

    fn is_complete(&self) -> Result<bool, DaemonError> {
        Ok(lock(&self.result, "operation result")?.is_some())
    }
}

struct OperationCompletionGuard {
    cell: Arc<OperationCell>,
    completed: bool,
}

impl OperationCompletionGuard {
    fn new(cell: Arc<OperationCell>) -> Self {
        Self {
            cell,
            completed: false,
        }
    }

    fn complete(&mut self, result: MutationResult) {
        self.cell.complete(result);
        self.completed = true;
    }
}

impl Drop for OperationCompletionGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.cell.complete(Err(outcome_unknown()));
        }
    }
}

struct SessionActor {
    id: SessionId,
    commands: SyncSender<SessionCommand>,
    cached: Mutex<ActorRuntimeSummary>,
    revision: watch::Receiver<Revision>,
    registry: Weak<RegistryInner>,
    limits: ResourceLimits,
    interrupt: TerminalDriverInterrupt,
    driver_ownership: TerminalDriverOwnership,
    end: Mutex<EndState>,
    end_changed: Condvar,
    registry_owned: AtomicBool,
    worker_done: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
    #[cfg(test)]
    panic_close: AtomicBool,
}

#[derive(Clone, Copy)]
struct ActorRuntimeSummary {
    has_controller: bool,
    viewport: TerminalSize,
    ended: bool,
}

#[derive(Default)]
struct EndState {
    requested: Option<SessionEndReason>,
    interrupt: InterruptState,
    result: Option<Result<(), DaemonError>>,
}

#[derive(Default)]
enum InterruptState {
    #[default]
    Idle,
    Pending,
    Ready(Result<(), DaemonError>),
}

enum CommandStart {
    Started,
    Expired,
    Ending,
}

struct SessionRuntime {
    driver: Option<TerminalDriver>,
    attachments: BTreeMap<AttachmentId, ActorAttachment>,
    resume: Option<RemoteResumeCheckpoint>,
    controller: Option<ControllerLease>,
    controller_operation: Option<ReplayOperationKey>,
    next_generation: u64,
    viewport: TerminalSize,
    last_revision: Revision,
}

struct ActorAttachment {
    principal: AttachmentPrincipal,
    #[cfg(unix)]
    resume_view_id: Option<ResumeViewId>,
    terminal: TerminalAttachment,
    ever_active: bool,
    prepared_snapshot_applied: bool,
    sync: AttachmentSync,
    lifecycle: watch::Sender<AttachmentLifecycle>,
    detached: Arc<AtomicBool>,
    final_update: FinalAttachmentUpdateSlot,
}

struct RemoteResumeCheckpoint {
    principal: AttachmentPrincipal,
    session_id: SessionId,
    view_id: ResumeViewId,
    revision: Revision,
    terminal: TerminalAttachment,
}

#[derive(Clone, Copy)]
enum AttachmentSync {
    Awaiting {
        revision: Revision,
        target: SyncTarget,
    },
    Active {
        generation: u64,
    },
    PreparedTakeover,
}

#[derive(Clone, Copy)]
enum SyncTarget {
    Active { generation: u64 },
    PreparedTakeover,
}

#[derive(Clone)]
struct CommandMeta {
    deadline: Instant,
    gate: Arc<CommandGate>,
}

struct CommandGate {
    state: AtomicU8,
}

impl Default for CommandGate {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(COMMAND_QUEUED),
        }
    }
}

impl CommandMeta {
    fn try_start(&self) -> bool {
        if Instant::now() >= self.deadline {
            let _ = self.gate.state.compare_exchange(
                COMMAND_QUEUED,
                COMMAND_EXPIRED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            return false;
        }
        self.gate
            .state
            .compare_exchange(
                COMMAND_QUEUED,
                COMMAND_STARTED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

/// One enqueued actor command whose result has not yet been awaited.
struct CommandWaiter<R> {
    response: Receiver<Result<R, DaemonError>>,
    gate: Arc<CommandGate>,
}

impl<R> CommandWaiter<R> {
    fn wait(self, deadline: Instant) -> Result<R, DaemonError> {
        wait_for_command_response(self.response, self.gate, deadline)
    }
}

enum SessionCommand {
    PrepareAttach {
        meta: CommandMeta,
        principal: AttachmentPrincipal,
        takeover: bool,
        resume: Option<RemoteResumeRequest>,
        reply: SyncSender<Result<PreparedAttachment, DaemonError>>,
    },
    #[cfg(unix)]
    DetachForRemoteResume {
        meta: CommandMeta,
        attachment_id: AttachmentId,
        reply: SyncSender<Result<bool, DaemonError>>,
    },
    SnapshotApplied {
        meta: CommandMeta,
        attachment_id: AttachmentId,
        revision: Revision,
        reply: SyncSender<Result<Option<TerminalSurfaceSnapshot>, DaemonError>>,
    },
    NextUpdate {
        meta: CommandMeta,
        attachment_id: AttachmentId,
        reply: SyncSender<Result<Option<AttachmentUpdate>, DaemonError>>,
    },
    #[cfg(unix)]
    FinalUpdate {
        meta: CommandMeta,
        attachment_id: AttachmentId,
        reply: SyncSender<Result<Option<AttachmentUpdate>, DaemonError>>,
    },
    SyncLatest {
        meta: CommandMeta,
        attachment_id: AttachmentId,
        known_revision: Revision,
        reply: SyncSender<Result<TerminalSurfaceSnapshot, DaemonError>>,
    },
    #[cfg(unix)]
    HistoryWindow {
        meta: CommandMeta,
        attachment_id: AttachmentId,
        query: TerminalHistoryWindowQuery,
        reply: SyncSender<Result<TerminalSurfaceHistoryWindowResult, DaemonError>>,
    },
    WriteInput {
        meta: CommandMeta,
        attachment_id: AttachmentId,
        bytes: Vec<u8>,
        reply: SyncSender<Result<(), DaemonError>>,
    },
    Resize {
        meta: CommandMeta,
        attachment_id: AttachmentId,
        size: TerminalSize,
        reply: SyncSender<Result<Revision, DaemonError>>,
    },
    Takeover {
        meta: CommandMeta,
        principal: AttachmentPrincipal,
        attachment_id: AttachmentId,
        operation_key: ReplayOperationKey,
        continuation: bool,
        reply: SyncSender<Result<(), DaemonError>>,
    },
    DetachRemotePrincipal {
        meta: CommandMeta,
        device_id: DeviceId,
        #[cfg(test)]
        processed: Option<SyncSender<()>>,
        reply: SyncSender<Result<PrincipalDetachOutcome, DaemonError>>,
    },
    #[cfg(unix)]
    CountRemoteAttachments {
        meta: CommandMeta,
        device_id: DeviceId,
        reply: SyncSender<Result<usize, DaemonError>>,
    },
    #[cfg(test)]
    BlockPtyEffect {
        meta: CommandMeta,
        entered: SyncSender<()>,
        release: Receiver<()>,
        executions: Arc<AtomicUsize>,
        reply: SyncSender<Result<(), DaemonError>>,
    },
    #[cfg(test)]
    PanicWorker,
    Wake,
}

impl SessionActor {
    fn start(
        id: SessionId,
        viewport: TerminalSize,
        driver: TerminalDriver,
        registry: Weak<RegistryInner>,
        limits: ResourceLimits,
    ) -> Result<Arc<Self>, DaemonError> {
        let revision = driver.revision_watch();
        let interrupt = driver.interrupt_handle();
        let driver_ownership = driver.ownership_handle();
        let (commands, receiver) = mpsc::sync_channel(SESSION_COMMAND_CAPACITY);
        let actor = Arc::new(Self {
            id,
            commands,
            cached: Mutex::new(ActorRuntimeSummary {
                has_controller: false,
                viewport,
                ended: false,
            }),
            revision,
            registry,
            limits,
            interrupt,
            driver_ownership,
            end: Mutex::new(EndState::default()),
            end_changed: Condvar::new(),
            registry_owned: AtomicBool::new(false),
            worker_done: AtomicBool::new(false),
            worker: Mutex::new(None),
            #[cfg(test)]
            panic_close: AtomicBool::new(false),
        });
        let startup = Arc::new(Mutex::new(Some(driver)));
        let thread_startup = Arc::clone(&startup);
        let thread_actor = Arc::clone(&actor);
        let spawned = thread::Builder::new()
            .name("zterm-session-actor".into())
            .spawn(move || {
                let mut finalizer = ActorWorkerFinalizer::new(Arc::clone(&thread_actor));
                let driver = thread_startup
                    .lock()
                    .ok()
                    .and_then(|mut driver| driver.take());
                let Some(driver) = driver else {
                    finalizer.complete(Err(synchronization_error("session actor startup")));
                    return;
                };
                let mut runtime = SessionRuntime {
                    driver: Some(driver),
                    attachments: BTreeMap::new(),
                    resume: None,
                    controller: None,
                    controller_operation: None,
                    next_generation: 0,
                    viewport,
                    last_revision: Revision::ZERO,
                };
                let result = catch_unwind(AssertUnwindSafe(|| {
                    run_session_actor(&thread_actor, &mut runtime, &receiver)
                }))
                .unwrap_or_else(|_| Err(outcome_unknown()));
                // If execution unwound before the ordinary explicit finalizer,
                // dropping the retained runtime hands its driver to the
                // nonblocking reaper. The actor finalizer below waits on the
                // driver's ownership signal before removing registry tokens.
                drop(runtime);
                finalizer.complete(result);
            });
        match spawned {
            Ok(worker) => {
                *actor
                    .worker
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(worker);
                Ok(actor)
            }
            Err(error) => {
                if let Some(driver) = lock(&startup, "session actor startup")?.take() {
                    let _ = driver.finalize_explicit();
                }
                Err(resource_error(format!(
                    "unable to start session actor thread: {error}"
                )))
            }
        }
    }

    fn request<R>(
        &self,
        deadline: Instant,
        build: impl FnOnce(CommandMeta, SyncSender<Result<R, DaemonError>>) -> SessionCommand,
    ) -> Result<R, DaemonError> {
        self.enqueue_command(deadline, build)?.wait(deadline)
    }

    /// Enqueues one command without waiting for its result. The returned
    /// [`CommandWaiter`] must be awaited to observe the exact outcome. This
    /// split lets a multi-actor operation admit every command before waiting
    /// for any of them, so one blocked actor cannot delay another's effect.
    fn enqueue_command<R>(
        &self,
        deadline: Instant,
        build: impl FnOnce(CommandMeta, SyncSender<Result<R, DaemonError>>) -> SessionCommand,
    ) -> Result<CommandWaiter<R>, DaemonError> {
        let gate = Arc::new(CommandGate::default());
        let meta = CommandMeta {
            deadline,
            gate: Arc::clone(&gate),
        };
        let (reply, response) = mpsc::sync_channel(1);
        let mut command = build(meta, reply);
        loop {
            match self.commands.try_send(command) {
                Ok(()) => break,
                Err(TrySendError::Disconnected(_)) => return Err(session_not_found()),
                Err(TrySendError::Full(returned)) => {
                    command = returned;
                    if Instant::now() >= deadline {
                        let _ = gate.state.compare_exchange(
                            COMMAND_QUEUED,
                            COMMAND_EXPIRED,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                        return Err(deadline_error(
                            "session command queue admission deadline elapsed",
                        ));
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
        Ok(CommandWaiter {
            response,
            gate: Arc::clone(&gate),
        })
    }

    fn runtime_summary(&self) -> Result<ActorRuntimeSummary, DaemonError> {
        let summary = *lock(&self.cached, "session actor summary")?;
        if summary.ended {
            Err(session_not_found())
        } else {
            Ok(summary)
        }
    }

    #[cfg(test)]
    fn block_pty_effect_for_test(
        &self,
        deadline: Instant,
        entered: SyncSender<()>,
        release: Receiver<()>,
        executions: Arc<AtomicUsize>,
    ) -> Result<(), DaemonError> {
        self.request(deadline, |meta, reply| SessionCommand::BlockPtyEffect {
            meta,
            entered,
            release,
            executions,
            reply,
        })
    }

    #[cfg(test)]
    fn panic_worker_for_test(&self, deadline: Instant) -> Result<(), DaemonError> {
        self.commands
            .try_send(SessionCommand::PanicWorker)
            .map_err(|_| resource_error("unable to inject actor worker panic"))?;
        while !self.worker_done() {
            if Instant::now() >= deadline {
                return Err(deadline_error("actor worker panic did not finalize"));
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    #[cfg(test)]
    fn panic_next_close_for_test(&self) {
        self.panic_close.store(true, Ordering::Release);
    }

    fn latest_revision(&self) -> Revision {
        *self.revision.borrow()
    }

    fn update_cached(&self, runtime: &SessionRuntime, ended: bool) {
        if let Ok(mut cached) = self.cached.lock() {
            *cached = ActorRuntimeSummary {
                has_controller: runtime.controller.is_some(),
                viewport: runtime.viewport,
                ended,
            };
        }
    }

    fn mark_registry_owned(self: &Arc<Self>) {
        self.registry_owned.store(true, Ordering::Release);
        if self.worker_done()
            && let Some(registry) = self.registry.upgrade()
        {
            registry.complete(self.id, self);
        }
    }

    fn begin_end(self: &Arc<Self>, reason: SessionEndReason) {
        let spawn_interrupt = {
            let Ok(mut end) = self.end.lock() else {
                return;
            };
            if end.result.is_some() {
                return;
            }
            if end.requested.is_none() {
                end.requested = Some(reason);
            }
            match end.interrupt {
                InterruptState::Idle | InterruptState::Ready(Err(_)) => {
                    end.interrupt = InterruptState::Pending;
                    true
                }
                InterruptState::Pending | InterruptState::Ready(Ok(())) => false,
            }
        };
        if spawn_interrupt {
            let actor = Arc::clone(self);
            let interrupt = self.interrupt.clone();
            let spawned = thread::Builder::new()
                .name("zterm-session-close".into())
                .spawn(move || {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        #[cfg(test)]
                        if actor.panic_close.swap(false, Ordering::AcqRel) {
                            panic!("injected session close panic");
                        }
                        interrupt
                            .close_explicitly()
                            .map(|_| ())
                            .map_err(map_driver_error)
                    }))
                    .unwrap_or_else(|_| Err(outcome_unknown()));
                    actor.finish_interrupt(result);
                });
            if let Err(error) = spawned {
                self.finish_interrupt(Err(resource_error(format!(
                    "unable to start session close thread: {error}"
                ))));
            }
        }
        let _ = self.commands.try_send(SessionCommand::Wake);
    }

    fn finish_interrupt(&self, result: Result<(), DaemonError>) {
        if let Ok(mut end) = self.end.lock() {
            end.interrupt = InterruptState::Ready(result);
        }
        self.end_changed.notify_all();
        let _ = self.commands.try_send(SessionCommand::Wake);
    }

    fn ready_end_reason(&self) -> Option<SessionEndReason> {
        let end = self.end.lock().ok()?;
        match (&end.requested, &end.interrupt) {
            (Some(reason), InterruptState::Ready(Ok(()))) => Some(reason.clone()),
            _ => None,
        }
    }

    fn requested_end_reason(&self) -> Option<SessionEndReason> {
        self.end.lock().ok()?.requested.clone()
    }

    fn try_start_command(&self, meta: &CommandMeta) -> CommandStart {
        let Ok(end) = self.end.lock() else {
            return CommandStart::Ending;
        };
        if end.requested.is_some() || end.result.is_some() {
            return CommandStart::Ending;
        }
        if meta.try_start() {
            CommandStart::Started
        } else {
            CommandStart::Expired
        }
    }

    fn finish_end(&self, result: Result<(), DaemonError>) {
        if let Ok(mut end) = self.end.lock() {
            end.result = Some(result);
        }
        self.end_changed.notify_all();
    }

    fn wait_finished(&self) -> Result<(), DaemonError> {
        self.wait_finished_until(Instant::now() + DEFAULT_SHUTDOWN_TIMEOUT)
    }

    fn wait_finished_until(&self, deadline: Instant) -> Result<(), DaemonError> {
        let mut end = lock(&self.end, "session completion")?;
        loop {
            if let Some(result) = end.result.clone() {
                return result;
            }
            if let InterruptState::Ready(Err(error)) = &end.interrupt {
                return Err(error.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(deadline_error(
                    "session cleanup did not finish before deadline",
                ));
            }
            let (next, timeout) = self
                .end_changed
                .wait_timeout(end, deadline.saturating_duration_since(now))
                .map_err(|_| synchronization_error("session completion"))?;
            end = next;
            if timeout.timed_out() && end.result.is_none() {
                return Err(deadline_error(
                    "session cleanup did not finish before deadline",
                ));
            }
        }
    }

    fn worker_done(&self) -> bool {
        self.worker_done.load(Ordering::Acquire)
    }

    fn join_finished(&self) -> Result<(), DaemonError> {
        if !self.worker_done() {
            return Err(deadline_error("session actor has not finished"));
        }
        let worker = lock(&self.worker, "session actor thread")?.take();
        if let Some(worker) = worker {
            worker
                .join()
                .map_err(|_| synchronization_error("session actor thread"))?;
        }
        Ok(())
    }

    fn join_finished_cleanup(&self) -> Result<(), DaemonError> {
        if !self.worker_done() {
            return Err(deadline_error("session actor has not finished"));
        }
        let worker = cleanup_lock(&self.worker).take();
        if let Some(worker) = worker {
            worker
                .join()
                .map_err(|_| synchronization_error("session actor thread"))?;
        }
        Ok(())
    }
}

impl Drop for SessionActor {
    fn drop(&mut self) {
        self.interrupt.interrupt();
        let worker = match self.worker.get_mut() {
            Ok(worker) => worker.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(worker) = worker {
            // Taking the handle is exclusive with `join_finished`; handing it
            // off is safe even when Drop runs on the actor worker itself.
            spawn_background_reaper("zterm-session-reaper", move || {
                let _ = worker.join();
            });
        }
    }
}

struct ActorWorkerFinalizer {
    actor: Arc<SessionActor>,
    completed: bool,
}

impl ActorWorkerFinalizer {
    fn new(actor: Arc<SessionActor>) -> Self {
        Self {
            actor,
            completed: false,
        }
    }

    fn complete(&mut self, result: Result<(), DaemonError>) {
        self.actor.driver_ownership.wait_released();
        self.actor.worker_done.store(true, Ordering::Release);
        self.actor.finish_end(result);
        if self.actor.registry_owned.load(Ordering::Acquire)
            && let Some(registry) = self.actor.registry.upgrade()
        {
            registry.complete(self.actor.id, &self.actor);
        }
        self.completed = true;
    }
}

impl Drop for ActorWorkerFinalizer {
    fn drop(&mut self) {
        if !self.completed {
            self.complete(Err(outcome_unknown()));
        }
    }
}

fn wait_for_command_response<R>(
    response: Receiver<Result<R, DaemonError>>,
    gate: Arc<CommandGate>,
    deadline: Instant,
) -> Result<R, DaemonError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    match response.recv_timeout(remaining) {
        Ok(result) => result,
        Err(RecvTimeoutError::Disconnected) => Err(session_not_found()),
        Err(RecvTimeoutError::Timeout) => {
            if gate
                .state
                .compare_exchange(
                    COMMAND_QUEUED,
                    COMMAND_EXPIRED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return Err(deadline_error(
                    "session command expired before starting its side effect",
                ));
            }
            if gate.state.load(Ordering::Acquire) == COMMAND_STARTED {
                return response.recv().map_err(|_| session_not_found())?;
            }
            Err(deadline_error(
                "session command expired before starting its side effect",
            ))
        }
    }
}

fn run_session_actor(
    actor: &Arc<SessionActor>,
    runtime: &mut SessionRuntime,
    commands: &Receiver<SessionCommand>,
) -> Result<(), DaemonError> {
    loop {
        reap_detached(actor, runtime)?;
        if let Some(reason) = actor.ready_end_reason() {
            return finish_runtime(actor, runtime, reason);
        }
        match poll_driver(runtime) {
            Ok(Some(reason)) => {
                let reason = actor.requested_end_reason().unwrap_or(reason);
                return finish_runtime(actor, runtime, reason);
            }
            Ok(None) => {}
            Err(_) => {
                if let Some(driver) = runtime.driver.as_ref()
                    && driver.close_explicitly().is_ok()
                {
                    return finish_runtime(actor, runtime, SessionEndReason::DriverFailure);
                }
            }
        }

        match commands.recv_timeout(SESSION_MONITOR_INTERVAL) {
            Ok(SessionCommand::Wake) => {}
            Ok(command) => dispatch_command(actor, runtime, command),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(driver) = runtime.driver.as_ref() {
                    let _ = driver.close_explicitly();
                }
                return finish_runtime(actor, runtime, SessionEndReason::DaemonStop);
            }
        }
    }
}

fn poll_driver(runtime: &SessionRuntime) -> Result<Option<SessionEndReason>, DaemonError> {
    let driver = runtime.driver.as_ref().ok_or_else(session_not_found)?;
    driver.check_health().map_err(map_driver_error)?;
    match driver.try_wait().map_err(map_driver_error)? {
        PtyChildState::Running => Ok(None),
        PtyChildState::Exited(status) => Ok(Some(SessionEndReason::NaturalExit {
            exit_code: status.exit_code(),
            signal: status.signal().map(ToOwned::to_owned),
        })),
    }
}

fn finish_runtime(
    actor: &Arc<SessionActor>,
    runtime: &mut SessionRuntime,
    reason: SessionEndReason,
) -> Result<(), DaemonError> {
    if let Some(driver) = runtime.driver.as_ref() {
        runtime.last_revision = driver.latest_revision();
    }
    runtime.resume = None;
    runtime.controller = None;
    runtime.controller_operation = None;
    if let Some(driver) = runtime.driver.as_ref() {
        driver.set_effect_target(None).map_err(map_driver_error)?;
    }
    let finalization = runtime
        .driver
        .take()
        .ok_or_else(session_not_found)
        .and_then(|driver| {
            driver
                .finalize_natural()
                .map(|_| ())
                .map_err(map_driver_error)
        });
    for attachment in runtime.attachments.values_mut() {
        let update = final_terminal_update(&mut attachment.terminal);
        if let Ok(mut final_update) = attachment.final_update.lock() {
            *final_update = Some(update);
        }
        attachment
            .lifecycle
            .send_replace(AttachmentLifecycle::SessionEnded(reason.clone()));
    }
    actor.update_cached(runtime, true);
    finalization
}

fn dispatch_command(
    actor: &Arc<SessionActor>,
    runtime: &mut SessionRuntime,
    command: SessionCommand,
) {
    match command {
        SessionCommand::PrepareAttach {
            meta,
            principal,
            takeover,
            resume,
            reply,
        } => respond(actor, meta, reply, || {
            prepare_attach(actor, runtime, principal, takeover, resume)
        }),
        #[cfg(unix)]
        SessionCommand::DetachForRemoteResume {
            meta,
            attachment_id,
            reply,
        } => respond(actor, meta, reply, || {
            detach_for_remote_resume(actor, runtime, attachment_id)
        }),
        SessionCommand::SnapshotApplied {
            meta,
            attachment_id,
            revision,
            reply,
        } => respond(actor, meta, reply, || {
            snapshot_applied(runtime, attachment_id, revision)
        }),
        SessionCommand::NextUpdate {
            meta,
            attachment_id,
            reply,
        } => respond(actor, meta, reply, || next_update(runtime, attachment_id)),
        #[cfg(unix)]
        SessionCommand::FinalUpdate {
            meta,
            attachment_id,
            reply,
        } => respond(actor, meta, reply, || final_update(runtime, attachment_id)),
        SessionCommand::SyncLatest {
            meta,
            attachment_id,
            known_revision,
            reply,
        } => respond(actor, meta, reply, || {
            sync_latest(runtime, attachment_id, known_revision)
        }),
        #[cfg(unix)]
        SessionCommand::HistoryWindow {
            meta,
            attachment_id,
            query,
            reply,
        } => respond(actor, meta, reply, || {
            history_window(runtime, attachment_id, query)
        }),
        SessionCommand::WriteInput {
            meta,
            attachment_id,
            bytes,
            reply,
        } => respond(actor, meta, reply, || {
            write_input(runtime, attachment_id, &bytes)
        }),
        SessionCommand::Resize {
            meta,
            attachment_id,
            size,
            reply,
        } => respond(actor, meta, reply, || {
            resize(actor, runtime, attachment_id, size)
        }),
        SessionCommand::Takeover {
            meta,
            principal,
            attachment_id,
            operation_key,
            continuation,
            reply,
        } => respond(actor, meta, reply, || {
            takeover(
                actor,
                runtime,
                principal,
                attachment_id,
                operation_key,
                continuation,
            )
        }),
        SessionCommand::DetachRemotePrincipal {
            meta,
            device_id,
            #[cfg(test)]
            processed,
            reply,
        } => respond(actor, meta, reply, || {
            let outcome = detach_remote_principal(actor, runtime, device_id)?;
            #[cfg(test)]
            if let Some(processed) = processed {
                let _ = processed.send(());
            }
            Ok(outcome)
        }),
        #[cfg(unix)]
        SessionCommand::CountRemoteAttachments {
            meta,
            device_id,
            reply,
        } => respond(actor, meta, reply, || {
            Ok(count_remote_attachments(runtime, device_id))
        }),
        #[cfg(test)]
        SessionCommand::BlockPtyEffect {
            meta,
            entered,
            release,
            executions,
            reply,
        } => respond(actor, meta, reply, || {
            executions.fetch_add(1, Ordering::AcqRel);
            entered
                .send(())
                .map_err(|_| synchronization_error("test PTY-effect entry"))?;
            release
                .recv()
                .map_err(|_| synchronization_error("test PTY-effect release"))?;
            Ok(())
        }),
        #[cfg(test)]
        SessionCommand::PanicWorker => panic!("injected session actor worker panic"),
        SessionCommand::Wake => {}
    }
}

fn respond<R>(
    actor: &SessionActor,
    meta: CommandMeta,
    reply: SyncSender<Result<R, DaemonError>>,
    operation: impl FnOnce() -> Result<R, DaemonError>,
) {
    let result = match actor.try_start_command(&meta) {
        CommandStart::Started => operation(),
        CommandStart::Expired => Err(deadline_error(
            "session command expired before starting its side effect",
        )),
        CommandStart::Ending => Err(session_not_found()),
    };
    let _ = reply.send(result);
}

fn prepare_attach(
    actor: &Arc<SessionActor>,
    runtime: &mut SessionRuntime,
    principal: AttachmentPrincipal,
    takeover: bool,
    resume: Option<RemoteResumeRequest>,
) -> Result<PreparedAttachment, DaemonError> {
    reap_detached(actor, runtime)?;
    #[cfg(unix)]
    let resume_view_id = resume.map(|request| request.view_id);
    let resumed_terminal = take_resume_terminal(actor, runtime, principal, resume);
    if runtime.controller.is_some() && !takeover {
        return Err(DaemonError::new(
            DomainErrorKind::SessionOccupied,
            "session already has a controller; use explicit takeover",
        ));
    }
    if runtime.controller.is_some()
        && runtime.attachments.values().any(|attachment| {
            matches!(
                attachment.sync,
                AttachmentSync::Awaiting {
                    target: SyncTarget::PreparedTakeover,
                    ..
                } | AttachmentSync::PreparedTakeover
            )
        })
    {
        return Err(DaemonError::new(
            DomainErrorKind::SessionOccupied,
            "session already has one pending takeover",
        ));
    }
    let attachment_id = next_attachment_id(&runtime.attachments)?;
    let driver = runtime.driver.as_ref().ok_or_else(session_not_found)?;
    let mut terminal = resumed_terminal.unwrap_or_else(|| driver.attach());
    let effect_broker = terminal.effect_broker();
    let effect_wakeup = effect_broker.subscribe();
    let revisions = terminal.revision_watch();
    let initial_state = if resume.is_some() && terminal.checkpoint_revision().is_some() {
        match terminal.sync_latest().map_err(map_driver_error)? {
            TerminalSurfaceDeltaResult::Delta(delta) => {
                let revision = delta.to_revision;
                let snapshot = terminal.latest_snapshot().map_err(map_driver_error)?;
                (snapshot, Some(delta), revision)
            }
            TerminalSurfaceDeltaResult::Resync(snapshot) => {
                let revision = snapshot.revision;
                (snapshot, None, revision)
            }
        }
    } else {
        terminal.discard_checkpoint();
        let snapshot = full_sync(&mut terminal)?;
        let revision = snapshot.revision;
        (snapshot, None, revision)
    };
    #[cfg(unix)]
    let (snapshot, initial_delta, initial_revision) = initial_state;
    #[cfg(not(unix))]
    let (snapshot, _, initial_revision) = initial_state;
    let target = if runtime.controller.is_none() {
        runtime.next_generation = runtime
            .next_generation
            .checked_add(1)
            .ok_or_else(|| resource_error("controller generation exhausted"))?;
        let generation = runtime.next_generation;
        runtime.controller = Some(ControllerLease {
            attachment_id,
            generation,
        });
        runtime.controller_operation = None;
        SyncTarget::Active { generation }
    } else {
        SyncTarget::PreparedTakeover
    };
    let (lifecycle, lifecycle_receiver) = watch::channel(AttachmentLifecycle::AwaitingSnapshot {
        revision: initial_revision,
    });
    let detached = Arc::new(AtomicBool::new(false));
    let final_update = Arc::new(Mutex::new(None));
    runtime.attachments.insert(
        attachment_id,
        ActorAttachment {
            principal,
            #[cfg(unix)]
            resume_view_id,
            terminal,
            ever_active: false,
            prepared_snapshot_applied: false,
            sync: AttachmentSync::Awaiting {
                revision: initial_revision,
                target,
            },
            lifecycle,
            detached: Arc::clone(&detached),
            final_update: Arc::clone(&final_update),
        },
    );
    reconcile_effect_target(runtime)?;
    actor.update_cached(runtime, false);
    Ok(PreparedAttachment {
        attachment: Arc::new(SessionAttachment {
            actor: Arc::clone(actor),
            attachment_id,
            detached,
            revisions,
            lifecycle: lifecycle_receiver,
            effect_broker,
            effect_wakeup,
            #[cfg(unix)]
            final_update,
        }),
        snapshot,
        #[cfg(unix)]
        initial_delta,
    })
}

fn take_resume_terminal(
    actor: &SessionActor,
    runtime: &mut SessionRuntime,
    principal: AttachmentPrincipal,
    request: Option<RemoteResumeRequest>,
) -> Option<TerminalAttachment> {
    let Some(request) = request else {
        runtime.resume = None;
        return None;
    };

    let candidate = runtime.resume.take();
    candidate.and_then(|candidate| {
        (candidate.principal == principal
            && candidate.session_id == actor.id
            && candidate.view_id == request.view_id
            && request.known_revision == Some(candidate.revision))
        .then_some(candidate.terminal)
    })
}

#[cfg(unix)]
fn detach_for_remote_resume(
    actor: &SessionActor,
    runtime: &mut SessionRuntime,
    attachment_id: AttachmentId,
) -> Result<bool, DaemonError> {
    let Some(attachment) = runtime.attachments.remove(&attachment_id) else {
        return Err(lease_lost());
    };
    attachment.detached.store(true, Ordering::Release);
    let active_controller = runtime
        .controller
        .is_some_and(|controller| controller.attachment_id == attachment_id)
        && matches!(attachment.sync, AttachmentSync::Active { .. });
    if runtime
        .controller
        .is_some_and(|controller| controller.attachment_id == attachment_id)
    {
        runtime.controller = None;
        runtime.controller_operation = None;
    }

    let saved = match (
        active_controller,
        attachment.principal,
        attachment.resume_view_id,
        attachment.terminal.checkpoint_revision(),
    ) {
        (
            true,
            principal @ AttachmentPrincipal::RemoteEndpoint { .. },
            Some(view_id),
            Some(revision),
        ) => {
            runtime.resume = Some(RemoteResumeCheckpoint {
                principal,
                session_id: actor.id,
                view_id,
                revision,
                terminal: attachment.terminal,
            });
            true
        }
        _ => {
            runtime.resume = None;
            false
        }
    };
    reconcile_effect_target(runtime)?;
    actor.update_cached(runtime, false);
    Ok(saved)
}

fn snapshot_applied(
    runtime: &mut SessionRuntime,
    attachment_id: AttachmentId,
    revision: Revision,
) -> Result<Option<TerminalSurfaceSnapshot>, DaemonError> {
    let attachment = runtime
        .attachments
        .get_mut(&attachment_id)
        .ok_or_else(lease_lost)?;
    let AttachmentSync::Awaiting {
        revision: expected,
        target,
    } = attachment.sync
    else {
        return Err(not_synchronized("attachment is not awaiting a snapshot"));
    };
    if revision != expected {
        attachment.terminal.discard_checkpoint();
        let snapshot = full_sync(&mut attachment.terminal)?;
        attachment.sync = AttachmentSync::Awaiting {
            revision: snapshot.revision,
            target,
        };
        attachment
            .lifecycle
            .send_replace(AttachmentLifecycle::AwaitingSnapshot {
                revision: snapshot.revision,
            });
        reconcile_effect_target(runtime)?;
        return Ok(Some(snapshot));
    }
    let lifecycle = match target {
        SyncTarget::Active { generation } => {
            attachment.ever_active = true;
            attachment.sync = AttachmentSync::Active { generation };
            AttachmentLifecycle::Active { generation }
        }
        SyncTarget::PreparedTakeover => {
            attachment.prepared_snapshot_applied = true;
            attachment.sync = AttachmentSync::PreparedTakeover;
            AttachmentLifecycle::PreparedTakeover
        }
    };
    reconcile_effect_target(runtime)?;
    runtime
        .attachments
        .get(&attachment_id)
        .ok_or_else(lease_lost)?
        .lifecycle
        .send_replace(lifecycle);
    Ok(None)
}

fn next_update(
    runtime: &mut SessionRuntime,
    attachment_id: AttachmentId,
) -> Result<Option<AttachmentUpdate>, DaemonError> {
    let attachment = runtime
        .attachments
        .get_mut(&attachment_id)
        .ok_or_else(lease_lost)?;
    let target = match attachment.sync {
        AttachmentSync::Active { generation } => SyncTarget::Active { generation },
        AttachmentSync::PreparedTakeover => SyncTarget::PreparedTakeover,
        AttachmentSync::Awaiting { .. } => return Ok(None),
    };
    match attachment
        .terminal
        .sync_changed()
        .map_err(map_driver_error)?
    {
        None => Ok(None),
        Some(TerminalSurfaceDeltaResult::Delta(delta)) => Ok(Some(AttachmentUpdate::Delta(delta))),
        Some(TerminalSurfaceDeltaResult::Resync(snapshot)) => {
            attachment.sync = AttachmentSync::Awaiting {
                revision: snapshot.revision,
                target,
            };
            attachment
                .lifecycle
                .send_replace(AttachmentLifecycle::AwaitingSnapshot {
                    revision: snapshot.revision,
                });
            reconcile_effect_target(runtime)?;
            Ok(Some(AttachmentUpdate::Snapshot(snapshot)))
        }
    }
}

#[cfg(unix)]
fn final_update(
    runtime: &mut SessionRuntime,
    attachment_id: AttachmentId,
) -> Result<Option<AttachmentUpdate>, DaemonError> {
    let attachment = runtime
        .attachments
        .get_mut(&attachment_id)
        .ok_or_else(lease_lost)?;
    Ok(attachment
        .terminal
        .sync_changed()
        .map_err(map_driver_error)?
        .map(semantic_update_value))
}

fn final_terminal_update(
    terminal: &mut TerminalAttachment,
) -> Result<Option<AttachmentUpdate>, DaemonError> {
    Ok(terminal
        .sync_changed()
        .map_err(map_driver_error)?
        .map(semantic_update_value))
}

fn semantic_update_value(update: TerminalSurfaceDeltaResult) -> AttachmentUpdate {
    match update {
        TerminalSurfaceDeltaResult::Delta(delta) => AttachmentUpdate::Delta(delta),
        TerminalSurfaceDeltaResult::Resync(snapshot) => AttachmentUpdate::Snapshot(snapshot),
    }
}

fn sync_latest(
    runtime: &mut SessionRuntime,
    attachment_id: AttachmentId,
    _known_revision: Revision,
) -> Result<TerminalSurfaceSnapshot, DaemonError> {
    let attachment = runtime
        .attachments
        .get_mut(&attachment_id)
        .ok_or_else(lease_lost)?;
    let target = match attachment.sync {
        AttachmentSync::Active { generation } => SyncTarget::Active { generation },
        AttachmentSync::PreparedTakeover => SyncTarget::PreparedTakeover,
        AttachmentSync::Awaiting { target, .. } => target,
    };
    attachment.terminal.discard_checkpoint();
    let snapshot = full_sync(&mut attachment.terminal)?;
    attachment.sync = AttachmentSync::Awaiting {
        revision: snapshot.revision,
        target,
    };
    attachment
        .lifecycle
        .send_replace(AttachmentLifecycle::AwaitingSnapshot {
            revision: snapshot.revision,
        });
    reconcile_effect_target(runtime)?;
    Ok(snapshot)
}

#[cfg(unix)]
fn history_window(
    runtime: &SessionRuntime,
    attachment_id: AttachmentId,
    query: TerminalHistoryWindowQuery,
) -> Result<TerminalSurfaceHistoryWindowResult, DaemonError> {
    require_existing_visual_sync_controller(runtime, attachment_id)?;
    runtime
        .attachments
        .get(&attachment_id)
        .ok_or_else(lease_lost)?
        .terminal
        .history_window(query)
        .map_err(map_driver_error)
}

fn write_input(
    runtime: &SessionRuntime,
    attachment_id: AttachmentId,
    bytes: &[u8],
) -> Result<(), DaemonError> {
    require_existing_visual_sync_controller(runtime, attachment_id)?;
    runtime
        .driver
        .as_ref()
        .ok_or_else(session_not_found)?
        .write_input(bytes)
        .map_err(map_driver_error)
}

fn resize(
    actor: &Arc<SessionActor>,
    runtime: &mut SessionRuntime,
    attachment_id: AttachmentId,
    size: TerminalSize,
) -> Result<Revision, DaemonError> {
    validate_viewport(actor.limits, size)?;
    require_resize_controller(runtime, attachment_id)?;
    let revision = runtime
        .driver
        .as_ref()
        .ok_or_else(session_not_found)?
        .resize(size)
        .map_err(map_driver_error)?;
    runtime.viewport = size;
    runtime.last_revision = revision;
    actor.update_cached(runtime, false);
    Ok(revision)
}

fn takeover(
    actor: &Arc<SessionActor>,
    runtime: &mut SessionRuntime,
    principal: AttachmentPrincipal,
    attachment_id: AttachmentId,
    operation_key: ReplayOperationKey,
    continuation: bool,
) -> Result<(), DaemonError> {
    let owner = runtime
        .attachments
        .get(&attachment_id)
        .map(|attachment| attachment.principal)
        .ok_or_else(lease_lost)?;
    if owner != principal {
        return Err(principal_mismatch());
    }
    if continuation
        && runtime
            .controller
            .is_some_and(|controller| controller.attachment_id == attachment_id)
    {
        require_existing_visual_sync_controller(runtime, attachment_id)?;
        return Ok(());
    }
    let prepared = runtime
        .attachments
        .get(&attachment_id)
        .is_some_and(|attachment| {
            matches!(attachment.sync, AttachmentSync::PreparedTakeover)
                || attachment.prepared_snapshot_applied
                    && matches!(
                        attachment.sync,
                        AttachmentSync::Awaiting {
                            target: SyncTarget::PreparedTakeover,
                            ..
                        }
                    )
        });
    if !prepared {
        return Err(not_synchronized(
            "takeover attachment has not applied its prepared snapshot",
        ));
    }
    if continuation {
        match runtime.controller {
            None => {}
            Some(_) if runtime.controller_operation == Some(operation_key) => {}
            Some(_) => return Err(outcome_unknown()),
        }
    }
    runtime.next_generation = runtime
        .next_generation
        .checked_add(1)
        .ok_or_else(|| resource_error("controller generation exhausted"))?;
    let generation = runtime.next_generation;
    let old_lifecycle = if let Some(old) = runtime.controller
        && old.attachment_id != attachment_id
        && let Some(mut attachment) = runtime.attachments.remove(&old.attachment_id)
    {
        attachment.terminal.discard_checkpoint();
        Some(attachment.lifecycle)
    } else {
        None
    };
    runtime.controller = Some(ControllerLease {
        attachment_id,
        generation,
    });
    runtime.controller_operation = Some(operation_key);
    let attachment = runtime
        .attachments
        .get_mut(&attachment_id)
        .ok_or_else(lease_lost)?;
    let active_lifecycle = match attachment.sync {
        AttachmentSync::PreparedTakeover => {
            attachment.sync = AttachmentSync::Active { generation };
            attachment.ever_active = true;
            Some(attachment.lifecycle.clone())
        }
        AttachmentSync::Awaiting {
            revision,
            target: SyncTarget::PreparedTakeover,
        } if attachment.prepared_snapshot_applied => {
            attachment.sync = AttachmentSync::Awaiting {
                revision,
                target: SyncTarget::Active { generation },
            };
            None
        }
        _ => unreachable!("takeover readiness was validated above"),
    };
    reconcile_effect_target(runtime)?;
    if let Some(lifecycle) = old_lifecycle {
        lifecycle.send_replace(AttachmentLifecycle::LeaseLost { generation });
    }
    if let Some(lifecycle) = active_lifecycle {
        lifecycle.send_replace(AttachmentLifecycle::Active { generation });
    }
    actor.update_cached(runtime, false);
    Ok(())
}

fn reap_detached(actor: &SessionActor, runtime: &mut SessionRuntime) -> Result<(), DaemonError> {
    let detached = runtime
        .attachments
        .iter()
        .filter_map(|(attachment_id, attachment)| {
            attachment
                .detached
                .load(Ordering::Acquire)
                .then_some(*attachment_id)
        })
        .collect::<Vec<_>>();
    if detached.is_empty() {
        return Ok(());
    }
    for attachment_id in detached {
        runtime.attachments.remove(&attachment_id);
        if runtime
            .controller
            .is_some_and(|lease| lease.attachment_id == attachment_id)
        {
            runtime.controller = None;
            runtime.controller_operation = None;
        }
    }
    reconcile_effect_target(runtime)?;
    actor.update_cached(runtime, false);
    Ok(())
}

fn detach_remote_principal(
    actor: &SessionActor,
    runtime: &mut SessionRuntime,
    device_id: DeviceId,
) -> Result<PrincipalDetachOutcome, DaemonError> {
    let resume_removed = runtime.resume.as_ref().is_some_and(|resume| {
        matches!(
            resume.principal,
            AttachmentPrincipal::RemoteEndpoint {
                device_id: remote,
                ..
            } if remote == device_id
        )
    });
    if resume_removed {
        runtime.resume = None;
    }
    let removed = runtime
        .attachments
        .iter()
        .filter(|(_, attachment)| {
            matches!(
                attachment.principal,
                AttachmentPrincipal::RemoteEndpoint {
                    device_id: remote,
                    ..
                } if remote == device_id
            )
        })
        .map(|(attachment_id, _)| *attachment_id)
        .collect::<Vec<_>>();

    let mut outcome = PrincipalDetachOutcome::default();
    for attachment_id in removed {
        if let Some(attachment) = runtime.attachments.remove(&attachment_id) {
            attachment.detached.store(true, Ordering::Release);
            outcome.attachments_removed += 1;
        }
        if runtime
            .controller
            .is_some_and(|lease| lease.attachment_id == attachment_id)
        {
            runtime.controller = None;
            runtime.controller_operation = None;
            outcome.controller_released = true;
        }
    }
    if outcome.attachments_removed > 0 || resume_removed {
        reconcile_effect_target(runtime)?;
        actor.update_cached(runtime, false);
    }
    Ok(outcome)
}

fn reconcile_effect_target(runtime: &SessionRuntime) -> Result<(), DaemonError> {
    let target = runtime.controller.and_then(|controller| {
        runtime
            .attachments
            .get(&controller.attachment_id)
            .filter(|attachment| {
                attachment.ever_active
                    && matches!(
                        attachment.sync,
                        AttachmentSync::Active { generation }
                            | AttachmentSync::Awaiting {
                                target: SyncTarget::Active { generation },
                                ..
                            } if generation == controller.generation
                    )
            })
            .map(|_| controller.attachment_id)
    });
    runtime
        .driver
        .as_ref()
        .ok_or_else(session_not_found)?
        .set_effect_target(target)
        .map_err(map_driver_error)
}

#[cfg(unix)]
fn count_remote_attachments(runtime: &SessionRuntime, device_id: DeviceId) -> usize {
    runtime
        .attachments
        .values()
        .filter(|attachment| {
            !attachment.detached.load(Ordering::Acquire)
                && matches!(
                    attachment.principal,
                    AttachmentPrincipal::RemoteEndpoint {
                        device_id: remote,
                        ..
                    } if remote == device_id
                )
        })
        .count()
}

fn require_resize_controller(
    runtime: &SessionRuntime,
    attachment_id: AttachmentId,
) -> Result<u64, DaemonError> {
    // Snapshot delivery and controller commands travel in opposite directions
    // on one duplex stream. Resize is replaceable controller state and may be
    // admitted while either the initial or a later Active snapshot is in
    // flight. Input and read-only presentation requests reuse this controller
    // check only after proving that the same attachment was active before its
    // current visual-sync window. First attach and takeover remain fenced.
    let attachment = runtime
        .attachments
        .get(&attachment_id)
        .ok_or_else(lease_lost)?;
    let generation = match attachment.sync {
        AttachmentSync::Active { generation } => generation,
        AttachmentSync::Awaiting {
            target: SyncTarget::Active { generation },
            ..
        } => generation,
        AttachmentSync::Awaiting {
            target: SyncTarget::PreparedTakeover,
            ..
        }
        | AttachmentSync::PreparedTakeover => {
            return Err(not_synchronized(
                "snapshot must be applied before controller operations",
            ));
        }
    };
    match runtime.controller {
        Some(ControllerLease {
            attachment_id: owner,
            generation: current,
        }) if owner == attachment_id && current == generation => Ok(generation),
        _ => Err(lease_lost()),
    }
}

fn require_existing_visual_sync_controller(
    runtime: &SessionRuntime,
    attachment_id: AttachmentId,
) -> Result<u64, DaemonError> {
    let attachment = runtime
        .attachments
        .get(&attachment_id)
        .ok_or_else(lease_lost)?;
    if matches!(attachment.sync, AttachmentSync::Awaiting { .. }) && !attachment.ever_active {
        return Err(not_synchronized(
            "snapshot must be applied before controller operations",
        ));
    }
    require_resize_controller(runtime, attachment_id)
}

fn full_sync(attachment: &mut TerminalAttachment) -> Result<TerminalSurfaceSnapshot, DaemonError> {
    match attachment.sync_latest().map_err(map_driver_error)? {
        TerminalSurfaceDeltaResult::Resync(snapshot) => Ok(snapshot),
        TerminalSurfaceDeltaResult::Delta(_) => Err(DaemonError::new(
            DomainErrorKind::MalformedFrame,
            "discarded attachment checkpoint did not produce a full snapshot",
        )),
    }
}

fn next_attachment_id(
    attachments: &BTreeMap<AttachmentId, ActorAttachment>,
) -> Result<AttachmentId, DaemonError> {
    for _ in 0..16 {
        let candidate = AttachmentId::from_array(random_16());
        if !attachments.contains_key(&candidate) {
            return Ok(candidate);
        }
    }
    Err(resource_error(
        "unable to allocate a unique attachment identity",
    ))
}

fn random_16() -> [u8; 16] {
    let secret = SecretKey::generate().to_bytes();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&secret[..16]);
    bytes
}

fn validate_viewport(limits: ResourceLimits, size: TerminalSize) -> Result<(), DaemonError> {
    if size.rows == 0
        || size.columns == 0
        || size.rows > limits.max_viewport_rows
        || size.columns > limits.max_viewport_columns
    {
        return Err(resource_error(format!(
            "viewport {}x{} exceeds the {}x{} bound",
            size.columns, size.rows, limits.max_viewport_columns, limits.max_viewport_rows
        )));
    }
    Ok(())
}

fn default_deadline() -> Instant {
    Instant::now() + DEFAULT_COMMAND_TIMEOUT
}

fn ensure_before_deadline(deadline: Instant) -> Result<(), DaemonError> {
    if Instant::now() >= deadline {
        Err(deadline_error("operation deadline elapsed before commit"))
    } else {
        Ok(())
    }
}

fn map_pty_error(error: PtyError) -> DaemonError {
    match error {
        PtyError::InvalidPath {
            kind: PtyPathKind::WorkingDirectory,
            ..
        } => DaemonError::new(
            DomainErrorKind::InvalidWorkingDirectory,
            "working directory is invalid or inaccessible",
        ),
        error @ PtyError::UnsupportedPlatform { .. } => {
            DaemonError::new(DomainErrorKind::UnsupportedPlatform, error.to_string())
        }
        error => DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string()),
    }
}

fn map_terminal_error(error: TerminalError) -> DaemonError {
    DaemonError::new(DomainErrorKind::ResourceExhausted, error.to_string())
}

fn map_driver_error(error: TerminalDriverError) -> DaemonError {
    match error {
        TerminalDriverError::Pty(error) => map_pty_error(error),
        TerminalDriverError::Terminal(error) => map_terminal_error(error),
        TerminalDriverError::Deadline(detail) => deadline_error(detail),
        error => DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string()),
    }
}

fn lock<'a, T>(mutex: &'a Mutex<T>, name: &'static str) -> Result<MutexGuard<'a, T>, DaemonError> {
    mutex.lock().map_err(|_| synchronization_error(name))
}

/// Ownership finalizers cannot abandon a child or reservation merely because
/// an earlier owner panicked while holding a mutex. Cleanup recovers the value,
/// clears poison for later diagnostics/retries, and still compare-checks the
/// exact ownership token before removing anything.
fn cleanup_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            mutex.clear_poison();
            poisoned.into_inner()
        }
    }
}

fn synchronization_error(name: &'static str) -> DaemonError {
    DaemonError::new(
        DomainErrorKind::StoreUnavailable,
        format!("{name} lock is poisoned"),
    )
}

fn reserved_main() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::ReservedSessionName,
        "main is reserved for the default create-if-missing session",
    )
}

fn session_already_exists(name: &SessionName) -> DaemonError {
    DaemonError::new(
        DomainErrorKind::SessionAlreadyExists,
        format!("session {name:?} already exists"),
    )
}

fn invalid_session(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::SessionNotFound, detail)
}

fn session_not_found() -> DaemonError {
    DaemonError::new(DomainErrorKind::SessionNotFound, "session is not live")
}

fn principal_mismatch() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::LeaseLost,
        "takeover attachment belongs to a different principal",
    )
}

fn not_synchronized(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::NotSynchronized, detail)
}

fn lease_lost() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::LeaseLost,
        "attachment does not own the current controller lease",
    )
}

fn resource_error(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::ResourceExhausted, detail)
}

fn deadline_error(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::DeadlineExceeded, detail)
}

fn outcome_unknown() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::OperationOutcomeUnknown,
        "operation result belongs to a retired epoch or evicted sequence",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use zterm_platform::pty::ExplicitPtyCommand;

    #[test]
    fn invalid_working_directory_error_redacts_the_host_path() {
        let secret_path = "/private/host/project/secret-cwd";
        let error = map_pty_error(PtyError::InvalidPath {
            kind: PtyPathKind::WorkingDirectory,
            path: secret_path.into(),
            issue: zterm_platform::pty::PtyPathIssue::NotFound,
        });

        assert_eq!(error.kind(), DomainErrorKind::InvalidWorkingDirectory);
        assert_eq!(
            error.detail(),
            "working directory is invalid or inaccessible"
        );
        assert!(!error.to_string().contains(secret_path));
    }

    #[test]
    fn session_summary_debug_redacts_working_directory_and_keeps_metadata() {
        let cwd_sentinel = "/private/tmp/DAEMON_CWD_SENTINEL_305e/project";
        let summary = SessionSummary {
            session_id: SessionId::from_array([0x5a; SessionId::LENGTH]),
            name: SessionName::new("debug-safe-session").expect("valid session name"),
            revision: Revision::new(59),
            has_controller: true,
            working_directory: cwd_sentinel.into(),
            viewport: TerminalSize::new(47, 163),
        };
        let fingerprint = OperationFingerprint::Create {
            name: summary.name.clone(),
            working_directory: Some(PathBuf::from(cwd_sentinel)),
            viewport: Some(summary.viewport),
        };

        let rendered = format!("{summary:?} {fingerprint:?}");
        assert!(!rendered.contains(cwd_sentinel));
        assert!(rendered.contains("[REDACTED]"));
        assert!(rendered.contains("debug-safe-session"));
        assert!(rendered.contains("has_controller: true"));
        assert!(rendered.contains("rows: 47"));
        assert!(rendered.contains("columns: 163"));
        assert!(rendered.contains("working_directory_present: true"));
        assert_eq!(summary.working_directory, PathBuf::from(cwd_sentinel));
        assert_eq!(summary, summary.clone());
        assert_eq!(fingerprint, fingerprint.clone());
        assert_ne!(
            fingerprint,
            OperationFingerprint::Create {
                name: summary.name.clone(),
                working_directory: Some(PathBuf::from("/private/tmp/a-different-cwd")),
                viewport: Some(summary.viewport),
            },
            "redacted Debug must not change cwd-sensitive replay equality"
        );
    }

    #[test]
    fn viewport_and_replay_identity_are_bounded() {
        let limits = ResourceLimits::default();
        assert!(validate_viewport(limits, TerminalSize::new(80, 240)).is_ok());
        assert_eq!(
            validate_viewport(limits, TerminalSize::new(81, 240))
                .expect_err("oversized viewport")
                .kind(),
            DomainErrorKind::ResourceExhausted
        );
        let device = DeviceId::from_array([7; 32]);
        let first = ReplayKey::new(
            AttachmentPrincipal::LocalSameUid {
                own_device_id: device,
                local_view_id: AttachmentId::from_array([1; 16]),
            },
            3,
        );
        let second = ReplayKey::new(
            AttachmentPrincipal::LocalSameUid {
                own_device_id: device,
                local_view_id: AttachmentId::from_array([2; 16]),
            },
            3,
        );
        assert_eq!(first, second, "local views share one stable device epoch");
    }

    #[test]
    fn replay_result_window_is_exact_and_recovers_after_oldest_completion() {
        let epoch = ReplayEpoch::default();
        let session_id = SessionId::from_array([0x31; SessionId::LENGTH]);
        let mut in_flight = Vec::with_capacity(OPERATION_RESULTS_PER_EPOCH);
        for sequence in 1..=OPERATION_RESULTS_PER_EPOCH {
            let sequence = u64::try_from(sequence).expect("result-window sequence fits u64");
            let ReplayRegistration::Execute(cell) = epoch
                .register(sequence, OperationFingerprint::Close { session_id })
                .expect("every production result slot is admitted")
            else {
                panic!("a fresh result slot must execute");
            };
            in_flight.push(cell);
        }

        let overflow_sequence =
            u64::try_from(OPERATION_RESULTS_PER_EPOCH + 1).expect("overflow sequence fits u64");
        let Err(error) = epoch.register(
            overflow_sequence,
            OperationFingerprint::Close { session_id },
        ) else {
            panic!("the next in-flight result must not exceed the exact window");
        };
        assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);

        in_flight[0].complete(Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            "completed oldest fixture result",
        )));
        let ReplayRegistration::Execute(recovered) = epoch
            .register(
                overflow_sequence,
                OperationFingerprint::Close { session_id },
            )
            .expect("completing the oldest result recovers one exact slot")
        else {
            panic!("the recovered result slot must execute");
        };
        let state = epoch.state.lock().expect("result-window state");
        assert_eq!(state.operations.len(), OPERATION_RESULTS_PER_EPOCH);
        assert_eq!(state.low_water, 2);
        drop(state);
        let Err(evicted) = epoch.register(1, OperationFingerprint::Close { session_id }) else {
            panic!("the reclaimed oldest result cannot execute again");
        };
        assert_eq!(evicted.kind(), DomainErrorKind::OperationOutcomeUnknown);
        recovered.complete(Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            "recovered fixture result",
        )));
    }

    #[test]
    fn replay_principal_limit_is_exact_and_a_new_incarnation_recovers_capacity() {
        let principal = |byte| AttachmentPrincipal::RemoteEndpoint {
            device_id: DeviceId::from_array([byte; DeviceId::LENGTH]),
            auth_generation: 1,
        };
        let mut replay = ReplayRegistry::new();
        let first_principal = principal(1);
        let mut first_lease = None;
        for index in 0..MAX_REPLAY_PRINCIPALS {
            let byte = u8::try_from(index + 1).expect("principal fixture fits one byte");
            let lease = replay
                .issue(principal(byte))
                .expect("every production replay-principal slot is admitted");
            if index == 0 {
                first_lease = Some(lease);
            }
        }
        assert_eq!(replay.issued_through.len(), MAX_REPLAY_PRINCIPALS);

        let overflow = principal(0xfe);
        let error = replay
            .issue(overflow)
            .expect_err("the next replay principal exceeds the daemon-lifetime bound");
        assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);
        let first_lease = first_lease.expect("first daemon lease retained");
        drop(replay);

        let mut restarted = ReplayRegistry::new();
        let mut restarted_incarnation = first_lease.daemon_incarnation.to_bytes();
        restarted_incarnation[0] ^= 0xff;
        restarted.incarnation = DaemonIncarnation::from_array(restarted_incarnation);
        let restarted_lease = restarted
            .issue(first_principal)
            .expect("a new daemon incarnation starts with recovered principal capacity");
        assert_eq!(restarted.issued_through.len(), 1);
        assert_eq!(restarted_lease.ordinal, first_lease.ordinal);
        assert_ne!(
            restarted_lease.daemon_incarnation,
            first_lease.daemon_incarnation,
        );
        let fingerprint = OperationFingerprint::Close {
            session_id: SessionId::from_array([0x32; SessionId::LENGTH]),
        };
        let Err(old_daemon) = restarted.register(
            ReplayKey::new(first_principal, first_lease.ordinal),
            OperationId {
                lease: first_lease,
                sequence: 1,
            },
            fingerprint.clone(),
        ) else {
            panic!("capacity recovery must not revive an old-daemon operation");
        };
        assert_eq!(old_daemon.kind(), DomainErrorKind::OperationOutcomeUnknown);
        let ReplayRegistration::Execute(restarted_cell) = restarted
            .register(
                ReplayKey::new(first_principal, restarted_lease.ordinal),
                OperationId {
                    lease: restarted_lease,
                    sequence: 1,
                },
                fingerprint,
            )
            .expect("the new-incarnation lease remains executable")
        else {
            panic!("the new-incarnation operation must execute");
        };
        restarted_cell.complete(Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            "new-incarnation fixture result",
        )));
    }

    #[test]
    fn replay_epochs_retire_with_an_exact_per_principal_floor() {
        let principal = AttachmentPrincipal::LocalSameUid {
            own_device_id: DeviceId::from_array([9; 32]),
            local_view_id: AttachmentId::from_array([1; 16]),
        };
        let mut replay = ReplayRegistry::new();
        let mut first = None;
        for _ in 0..=MAX_ACTIVE_OPERATION_EPOCHS {
            let lease = replay.issue(principal).expect("lease is daemon-issued");
            first.get_or_insert(lease);
            let registration = replay
                .register(
                    ReplayKey::new(principal, lease.ordinal),
                    OperationId { lease, sequence: 1 },
                    OperationFingerprint::Close {
                        session_id: SessionId::from_array([1; 16]),
                    },
                )
                .expect("issued lease is admitted");
            let ReplayRegistration::Execute(cell) = registration else {
                panic!("new epoch must execute");
            };
            cell.complete(Err(DaemonError::new(
                DomainErrorKind::Cancelled,
                "fixture result",
            )));
        }
        assert_eq!(replay.epochs.len(), MAX_ACTIVE_OPERATION_EPOCHS);
        let first = first.expect("first lease");
        let retired = replay.register(
            ReplayKey::new(principal, first.ordinal),
            OperationId {
                lease: first,
                sequence: 1,
            },
            OperationFingerprint::Close {
                session_id: SessionId::from_array([1; 16]),
            },
        );
        let Err(error) = retired else {
            panic!("retired epoch cannot execute again");
        };
        assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
    }

    #[test]
    fn replay_never_retires_an_in_flight_result_to_admit_a_new_epoch() {
        let principal = AttachmentPrincipal::LocalSameUid {
            own_device_id: DeviceId::from_array([8; 32]),
            local_view_id: AttachmentId::from_array([2; 16]),
        };
        let mut replay = ReplayRegistry::new();
        let mut pending = Vec::new();
        let mut leases = Vec::new();
        for _ in 0..MAX_ACTIVE_OPERATION_EPOCHS {
            let lease = replay.issue(principal).expect("lease is daemon-issued");
            let ReplayRegistration::Execute(cell) = replay
                .register(
                    ReplayKey::new(principal, lease.ordinal),
                    OperationId { lease, sequence: 1 },
                    OperationFingerprint::Close {
                        session_id: SessionId::from_array([2; 16]),
                    },
                )
                .expect("in-flight epoch is admitted")
            else {
                panic!("new epoch must execute");
            };
            leases.push(lease);
            pending.push(cell);
        }

        let error = replay
            .issue(principal)
            .expect_err("an in-flight result cannot be made unknowable");
        assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);

        pending[0].complete(Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            "completed fixture result",
        )));
        let next_lease = replay
            .issue(principal)
            .expect("a completed oldest epoch can retire");
        let ReplayRegistration::Execute(new_cell) = replay
            .register(
                ReplayKey::new(principal, next_lease.ordinal),
                OperationId {
                    lease: next_lease,
                    sequence: 1,
                },
                OperationFingerprint::Close {
                    session_id: SessionId::from_array([2; 16]),
                },
            )
            .expect("newly issued epoch executes")
        else {
            panic!("the new epoch must execute");
        };
        new_cell.complete(Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            "new fixture result",
        )));
        let old = replay.register(
            ReplayKey::new(principal, leases[0].ordinal),
            OperationId {
                lease: leases[0],
                sequence: 1,
            },
            OperationFingerprint::Close {
                session_id: SessionId::from_array([2; 16]),
            },
        );
        let Err(error) = old else {
            panic!("a retired operation must never be registered for execution");
        };
        assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
    }

    #[test]
    fn replay_rejects_unissued_and_old_daemon_leases_without_mutating_state() {
        let principal = AttachmentPrincipal::LocalSameUid {
            own_device_id: DeviceId::from_array([19; 32]),
            local_view_id: AttachmentId::from_array([3; 16]),
        };
        let mut first_daemon = ReplayRegistry::new();
        let lease = first_daemon.issue(principal).expect("issued lease");
        let mut restarted = ReplayRegistry::new();
        let before = (
            restarted.epochs.len(),
            restarted.retired_through.len(),
            restarted.issued_through.len(),
        );
        let operation = OperationId { lease, sequence: 1 };
        let fingerprint = OperationFingerprint::Close {
            session_id: SessionId::from_array([3; 16]),
        };
        let old = restarted.register(
            ReplayKey::new(principal, lease.ordinal),
            operation,
            fingerprint.clone(),
        );
        let Err(old) = old else {
            panic!("restart rejects old lease");
        };
        assert_eq!(old.kind(), DomainErrorKind::OperationOutcomeUnknown);
        assert_eq!(
            before,
            (
                restarted.epochs.len(),
                restarted.retired_through.len(),
                restarted.issued_through.len(),
            )
        );

        let issued = restarted.issue(principal).expect("new daemon lease");
        let invented = OperationId {
            lease: OperationLease {
                ordinal: issued.ordinal + 10,
                ..issued
            },
            sequence: 1,
        };
        let Err(error) = restarted.register(
            ReplayKey::new(principal, invented.lease.ordinal),
            invented,
            fingerprint,
        ) else {
            panic!("invented high lease is rejected");
        };
        assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
        assert_eq!(
            restarted.issued_through[&ReplayPrincipal::from_attachment(principal)],
            1
        );
    }

    #[test]
    fn lost_empty_leases_retire_boundedly_and_ordinal_exhaustion_is_explicit() {
        let principal = AttachmentPrincipal::LocalSameUid {
            own_device_id: DeviceId::from_array([21; 32]),
            local_view_id: AttachmentId::from_array([5; 16]),
        };
        let mut replay = ReplayRegistry::new();
        let first = replay.issue(principal).expect("first empty lease");
        let mut latest = first;
        for _ in 0..(MAX_ACTIVE_OPERATION_EPOCHS * 3) {
            latest = replay
                .issue(principal)
                .expect("lost empty lease remains bounded");
        }
        assert_eq!(replay.epochs.len(), MAX_ACTIVE_OPERATION_EPOCHS);
        let Err(retired) = replay.register(
            ReplayKey::new(principal, first.ordinal),
            OperationId {
                lease: first,
                sequence: 1,
            },
            OperationFingerprint::Close {
                session_id: SessionId::from_array([5; 16]),
            },
        ) else {
            panic!("old empty lease was retired");
        };
        assert_eq!(retired.kind(), DomainErrorKind::OperationOutcomeUnknown);
        let ReplayRegistration::Execute(cell) = replay
            .register(
                ReplayKey::new(principal, latest.ordinal),
                OperationId {
                    lease: latest,
                    sequence: u64::MAX - 1,
                },
                OperationFingerprint::Close {
                    session_id: SessionId::from_array([5; 16]),
                },
            )
            .expect("latest issued lease executes")
        else {
            panic!("latest operation executes");
        };
        cell.complete(Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            "fixture result",
        )));

        let replay_principal = ReplayPrincipal::from_attachment(principal);
        replay.issued_through.insert(replay_principal, u64::MAX);
        let exhausted = replay.issue(principal).expect_err("ordinal never wraps");
        assert_eq!(exhausted.kind(), DomainErrorKind::ResourceExhausted);
    }

    #[test]
    fn read_only_session_queries_allocate_no_operation_lease_state() {
        let service = SessionService::with_spawner(
            DeviceId::from_array([22; 32]),
            ResourceLimits::default(),
            |_size, _cwd| panic!("read-only query must not spawn"),
        );
        assert!(service.list().expect("empty list").is_empty());
        let replay = service.replay.lock().expect("replay state");
        assert!(replay.epochs.is_empty());
        assert!(replay.issued_through.is_empty());
        assert!(replay.retired_through.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_remote_resume_cell_moves_the_visible_checkpoint_or_falls_back_to_snapshot() {
        use std::io::Write as _;

        use nix::sys::stat::Mode;

        let temporary = tempfile::tempdir().expect("temporary resume-cell fixture");
        let gate = temporary.path().join("gate");
        nix::unistd::mkfifo(&gate, Mode::S_IRUSR | Mode::S_IWUSR)
            .expect("create child-output gate");
        let service = unix_fixture_service(
            DeviceId::from_array([0x81; 32]),
            temporary.path().to_path_buf(),
            "printf 'READY\\r\\n'; IFS= read -r _ < gate; printf 'LATER\\r\\n'; exec /bin/cat",
        );
        let local = service.local_principal(AttachmentId::from_array([0x81; 16]));
        let lease = service
            .issue_operation_lease(local)
            .expect("fixture operation lease");
        let summary = service
            .create(
                local,
                OperationId { lease, sequence: 1 },
                SessionName::new("resume-cell").expect("fixture session name"),
                None,
                None,
            )
            .expect("fixture session creates");
        let remote = AttachmentPrincipal::RemoteEndpoint {
            device_id: DeviceId::from_array([0x82; 32]),
            auth_generation: 7,
        };
        let view_id = ResumeViewId::from_array([0x83; 16]);
        let request = |known_revision| RemoteAttachmentRequest {
            selector: Some(SessionSelector::Id(summary.session_id)),
            create_main: false,
            takeover: false,
            initial_viewport: None,
            resume: RemoteResumeRequest {
                view_id,
                known_revision,
            },
        };

        let first = service
            .prepare_remote_attach_until(
                remote,
                request(None),
                Instant::now() + Duration::from_secs(2),
            )
            .expect("initial remote attachment");
        assert!(first.initial_delta.is_none());
        assert!(
            first
                .attachment
                .snapshot_applied(first.snapshot.revision)
                .expect("activate first remote attachment")
                .is_none()
        );
        let baseline = wait_for_attachment_text(&first, b"READY").await;
        let first_attachment_id = first.attachment.attachment_id();
        let overlap_error = match service.prepare_remote_attach_until(
            remote,
            request(Some(baseline)),
            Instant::now() + Duration::from_secs(2),
        ) {
            Ok(_) => panic!("a live view cannot be resumed before transport EOF"),
            Err(error) => error,
        };
        assert_eq!(overlap_error.kind(), DomainErrorKind::SessionOccupied);
        first
            .attachment
            .write_input(b"")
            .expect("the original controller survives an overlapping resume request");
        let mut revisions = first
            .attachment
            .revision_watch()
            .expect("revision watermark");
        let _ = revisions.borrow_and_update();
        assert!(
            first
                .attachment
                .detach_for_remote_resume_until(Instant::now() + Duration::from_secs(2))
                .expect("transport EOF saves the exact remote controller checkpoint")
        );
        assert!(
            !service
                .list()
                .expect("controller release observation")
                .into_iter()
                .find(|candidate| candidate.session_id == summary.session_id)
                .expect("fixture session remains live")
                .has_controller
        );

        let gate_write = tokio::task::spawn_blocking(move || {
            let mut gate = std::fs::OpenOptions::new()
                .write(true)
                .open(gate)
                .expect("open child-output gate");
            gate.write_all(b"continue\n").expect("release child output");
            gate.flush().expect("flush child-output gate");
        });
        tokio::time::timeout(Duration::from_secs(2), gate_write)
            .await
            .expect("child opened its output gate")
            .expect("gate writer task");
        tokio::time::timeout(Duration::from_secs(2), async {
            while *revisions.borrow_and_update() <= baseline {
                revisions
                    .changed()
                    .await
                    .expect("driver revision remains open");
            }
        })
        .await
        .expect("post-disconnect output reached the authoritative model");

        let resumed = service
            .prepare_remote_attach_until(
                remote,
                request(Some(baseline)),
                Instant::now() + Duration::from_secs(2),
            )
            .expect("matching remote checkpoint resumes");
        assert_ne!(resumed.attachment.attachment_id(), first_attachment_id);
        let delta = resumed
            .initial_delta
            .as_ref()
            .expect("exact checkpoint produces one merged semantic delta");
        assert_eq!(delta.from_revision, baseline);
        assert!(delta.to_revision > baseline);
        assert!(terminal_delta_contains(delta, "LATER"));
        let resumed_revision = delta.to_revision;
        assert!(
            resumed
                .attachment
                .snapshot_applied(resumed_revision)
                .expect("acknowledge resume delta")
                .is_none()
        );
        assert!(
            resumed
                .attachment
                .detach_for_remote_resume_until(Instant::now() + Duration::from_secs(2))
                .expect("second transport EOF replaces the sole resume cell")
        );

        let mismatched = service
            .prepare_remote_attach_until(
                remote,
                request(Some(Revision::ZERO)),
                Instant::now() + Duration::from_secs(2),
            )
            .expect("a mismatched baseline falls back to an authoritative snapshot");
        assert!(mismatched.initial_delta.is_none());
        assert!(terminal_snapshot_contains(&mismatched.snapshot, b"LATER"));
        assert!(
            mismatched
                .attachment
                .snapshot_applied(mismatched.snapshot.revision)
                .expect("activate fallback snapshot")
                .is_none()
        );
        mismatched.attachment.detach();

        let after_explicit_detach = service
            .prepare_remote_attach_until(
                remote,
                request(Some(mismatched.snapshot.revision)),
                Instant::now() + Duration::from_secs(2),
            )
            .expect("explicit detach leaves no resumable controller checkpoint");
        assert!(after_explicit_detach.initial_delta.is_none());
        after_explicit_detach.attachment.detach();
        service.shutdown().expect("resume fixture shuts down");
    }

    #[cfg(unix)]
    #[test]
    fn resumed_attachment_effect_eligibility_waits_for_its_current_active_barrier() {
        let temporary = tempfile::tempdir().expect("temporary resume-effect fixture");
        let service = unix_fixture_service(
            DeviceId::from_array([0xa1; 32]),
            temporary.path().to_path_buf(),
            "exec /bin/cat",
        );
        let local = service.local_principal(AttachmentId::from_array([0xa1; 16]));
        let lease = service
            .issue_operation_lease(local)
            .expect("fixture operation lease");
        let summary = service
            .create(
                local,
                OperationId { lease, sequence: 1 },
                SessionName::new("resume-effect-fence").expect("fixture session name"),
                None,
                None,
            )
            .expect("fixture session creates");
        let remote = AttachmentPrincipal::RemoteEndpoint {
            device_id: DeviceId::from_array([0xa2; 32]),
            auth_generation: 11,
        };
        let view_id = ResumeViewId::from_array([0xa3; 16]);
        let request = |known_revision| RemoteAttachmentRequest {
            selector: Some(SessionSelector::Id(summary.session_id)),
            create_main: false,
            takeover: false,
            initial_viewport: None,
            resume: RemoteResumeRequest {
                view_id,
                known_revision,
            },
        };

        let first = service
            .prepare_remote_attach_until(
                remote,
                request(None),
                Instant::now() + Duration::from_secs(2),
            )
            .expect("initial remote attachment");
        assert!(
            first
                .attachment
                .snapshot_applied(first.snapshot.revision)
                .expect("activate initial controller")
                .is_none()
        );
        let retained_revision = first.snapshot.revision;
        assert!(
            first
                .attachment
                .detach_for_remote_resume_until(Instant::now() + Duration::from_secs(2))
                .expect("save active controller checkpoint")
        );

        let resumed = service
            .prepare_remote_attach_until(
                remote,
                request(Some(retained_revision)),
                Instant::now() + Duration::from_secs(2),
            )
            .expect("matching checkpoint resumes into a new attachment lifetime");
        let before_active = TerminalHostEffect::ClipboardWrite(
            zterm_core::terminal::TerminalClipboardWrite::new("resume before active".to_owned())
                .expect("valid clipboard fixture"),
        );
        resumed
            .attachment
            .effect_broker
            .publish_for_test(before_active)
            .expect("publish through the real Session broker");
        assert!(
            resumed
                .attachment
                .take_host_effect()
                .expect("inspect resumed attachment effect")
                .is_none(),
            "a retained checkpoint must not transfer effect eligibility into a new attachment lifetime"
        );

        assert!(
            resumed
                .attachment
                .snapshot_applied(resumed.snapshot.revision)
                .expect("activate resumed attachment")
                .is_none()
        );
        let after_active = TerminalHostEffect::ClipboardWrite(
            zterm_core::terminal::TerminalClipboardWrite::new("resume after active".to_owned())
                .expect("valid clipboard fixture"),
        );
        resumed
            .attachment
            .effect_broker
            .publish_for_test(after_active)
            .expect("publish after current Active barrier");
        let TerminalHostEffect::ClipboardWrite(write) = resumed
            .attachment
            .take_host_effect()
            .expect("take resumed attachment effect")
            .expect("active resumed attachment is eligible");
        assert_eq!(write.as_str(), "resume after active");

        resumed.attachment.detach();
        service
            .shutdown()
            .expect("resume-effect fixture shuts down");
    }

    #[cfg(unix)]
    #[test]
    fn takeover_awaiting_replacement_effect_eligibility_waits_for_active_barrier() {
        let temporary = tempfile::tempdir().expect("temporary takeover-effect fixture");
        let service = unix_fixture_service(
            DeviceId::from_array([0xb1; 32]),
            temporary.path().to_path_buf(),
            "exec /bin/cat",
        );
        let original = service.local_principal(AttachmentId::from_array([0xb1; 16]));
        let original_lease = service
            .issue_operation_lease(original)
            .expect("original operation lease");
        let summary = service
            .create(
                original,
                OperationId {
                    lease: original_lease,
                    sequence: 1,
                },
                SessionName::new("takeover-effect-fence").expect("fixture session name"),
                None,
                None,
            )
            .expect("fixture session creates");
        let active = service
            .prepare_attach(
                original,
                Some(SessionSelector::Id(summary.session_id)),
                false,
                false,
                None,
            )
            .expect("original controller prepares");
        assert!(
            active
                .attachment
                .snapshot_applied(active.snapshot.revision)
                .expect("activate original controller")
                .is_none()
        );

        let replacement_principal = service.local_principal(AttachmentId::from_array([0xb2; 16]));
        let replacement_lease = service
            .issue_operation_lease(replacement_principal)
            .expect("replacement operation lease");
        let replacement = service
            .prepare_attach(
                replacement_principal,
                Some(SessionSelector::Id(summary.session_id)),
                false,
                true,
                None,
            )
            .expect("takeover attachment prepares");
        assert!(
            replacement
                .attachment
                .snapshot_applied(replacement.snapshot.revision)
                .expect("acknowledge prepared takeover snapshot")
                .is_none()
        );
        let current_snapshot = replacement
            .attachment
            .sync_latest(replacement.snapshot.revision)
            .expect("begin a replacement snapshot before takeover response");
        service
            .takeover(
                replacement_principal,
                OperationId {
                    lease: replacement_lease,
                    sequence: 1,
                },
                &replacement.attachment,
            )
            .expect("commit takeover while current replacement is awaiting acknowledgement");

        let before_active = TerminalHostEffect::ClipboardWrite(
            zterm_core::terminal::TerminalClipboardWrite::new("takeover before active".to_owned())
                .expect("valid clipboard fixture"),
        );
        replacement
            .attachment
            .effect_broker
            .publish_for_test(before_active)
            .expect("publish through the real Session broker");
        assert!(
            replacement
                .attachment
                .take_host_effect()
                .expect("inspect takeover attachment effect")
                .is_none(),
            "takeover commit must not bypass the current attachment lifetime's Active barrier"
        );

        assert!(
            replacement
                .attachment
                .snapshot_applied(current_snapshot.revision)
                .expect("activate replacement controller")
                .is_none()
        );
        let after_active = TerminalHostEffect::ClipboardWrite(
            zterm_core::terminal::TerminalClipboardWrite::new("takeover after active".to_owned())
                .expect("valid clipboard fixture"),
        );
        replacement
            .attachment
            .effect_broker
            .publish_for_test(after_active)
            .expect("publish after replacement Active barrier");
        let TerminalHostEffect::ClipboardWrite(write) = replacement
            .attachment
            .take_host_effect()
            .expect("take replacement attachment effect")
            .expect("active takeover attachment is eligible");
        assert_eq!(write.as_str(), "takeover after active");

        replacement.attachment.detach();
        service
            .shutdown()
            .expect("takeover-effect fixture shuts down");
    }

    #[cfg(unix)]
    #[test]
    fn semantic_history_and_input_obey_the_controller_sync_window() {
        let temporary = tempfile::tempdir().expect("temporary visual-sync fixture");
        let service = unix_fixture_service(
            DeviceId::from_array([0x91; 32]),
            temporary.path().to_path_buf(),
            "exec /bin/cat",
        );
        let controller = service.local_principal(AttachmentId::from_array([0x91; 16]));
        let lease = service
            .issue_operation_lease(controller)
            .expect("fixture operation lease");
        let summary = service
            .create(
                controller,
                OperationId { lease, sequence: 1 },
                SessionName::new("visual-sync-window").expect("fixture session name"),
                None,
                None,
            )
            .expect("fixture session creates");
        let prepared = service
            .prepare_attach(
                controller,
                Some(SessionSelector::Id(summary.session_id)),
                false,
                false,
                None,
            )
            .expect("controller attachment prepares");
        let metrics = prepared
            .snapshot
            .surface
            .scroll_metrics
            .expect("main-screen snapshot carries history coordinates");
        let query = TerminalHistoryWindowQuery {
            anchor: TerminalHistoryWindowAnchor {
                epoch: metrics.epoch,
                revision: metrics.revision,
                max_offset_from_bottom: metrics.max_offset_from_bottom,
                viewport: prepared.snapshot.surface.size,
            },
            target_offset_from_bottom: 0,
            older_margin_rows: 0,
            newer_margin_rows: 0,
        };
        let error = prepared
            .attachment
            .history_window_until(query, Instant::now() + Duration::from_secs(1))
            .expect_err("the first snapshot must be acknowledged before history projection");
        assert_eq!(error.kind(), DomainErrorKind::NotSynchronized);
        let error = prepared
            .attachment
            .write_input(b"must-not-reach-initial-pty")
            .expect_err("the first snapshot must be acknowledged before input");
        assert_eq!(error.kind(), DomainErrorKind::NotSynchronized);
        assert!(
            prepared
                .attachment
                .snapshot_applied(prepared.snapshot.revision)
                .expect("activate controller")
                .is_none()
        );
        let replacement = prepared
            .attachment
            .sync_latest(prepared.snapshot.revision)
            .expect("enter deterministic visual synchronization");
        assert!(matches!(
            prepared
                .attachment
                .history_window_until(query, Instant::now() + Duration::from_secs(1))
                .expect("an existing controller may read history during resynchronization"),
            TerminalSurfaceHistoryWindowResult::Frame(_)
        ));
        prepared
            .attachment
            .write_input(b"")
            .expect("existing controller input crosses its visual-sync window");
        assert!(
            prepared
                .attachment
                .snapshot_applied(replacement.revision)
                .expect("acknowledge replacement")
                .is_none()
        );
        service.shutdown().expect("fixture session shuts down");
    }

    #[cfg(unix)]
    #[test]
    fn remote_attachment_count_is_directional_and_read_only() {
        let temporary = tempfile::tempdir().expect("temporary attachment-count fixture");
        let service = unix_fixture_service(
            DeviceId::from_array([0x71; 32]),
            temporary.path().to_path_buf(),
            "exec /bin/cat",
        );
        let local = service.local_principal(AttachmentId::from_array([0x71; 16]));
        let lease = service
            .issue_operation_lease(local)
            .expect("fixture operation lease");
        let create = |sequence, name: &str| {
            service
                .create(
                    local,
                    OperationId { lease, sequence },
                    SessionName::new(name).expect("fixture session name"),
                    None,
                    None,
                )
                .expect("fixture session creates")
        };
        let first = create(1, "remote-count-a");
        let second = create(2, "remote-count-b");
        let other = create(3, "remote-count-c");
        let target_id = DeviceId::from_array([0x72; 32]);
        let target = AttachmentPrincipal::RemoteEndpoint {
            device_id: target_id,
            auth_generation: 4,
        };
        let other_id = DeviceId::from_array([0x73; 32]);
        let first_attachment = service
            .prepare_attach(
                target,
                Some(SessionSelector::Id(first.session_id)),
                false,
                false,
                None,
            )
            .expect("first remote attachment");
        let second_attachment = service
            .prepare_attach(
                target,
                Some(SessionSelector::Id(second.session_id)),
                false,
                false,
                None,
            )
            .expect("second remote attachment");
        let other_attachment = service
            .prepare_attach(
                AttachmentPrincipal::RemoteEndpoint {
                    device_id: other_id,
                    auth_generation: 9,
                },
                Some(SessionSelector::Id(other.session_id)),
                false,
                false,
                None,
            )
            .expect("other remote attachment");

        let deadline = Instant::now() + Duration::from_secs(2);
        assert_eq!(
            service
                .remote_attachment_count_until(target_id, deadline)
                .expect("target count"),
            2
        );
        assert_eq!(
            service
                .remote_attachment_count_until(other_id, deadline)
                .expect("other count"),
            1
        );
        assert_eq!(service.list().expect("sessions remain live").len(), 3);

        for prepared in [&first_attachment, &second_attachment, &other_attachment] {
            assert!(
                prepared
                    .attachment
                    .snapshot_applied(prepared.snapshot.revision)
                    .expect("count leaves attachment usable")
                    .is_none()
            );
        }
        service.shutdown().expect("fixture sessions shut down");
    }

    #[test]
    fn panicking_execution_completes_duplicate_waiters_with_outcome_unknown() {
        let service = SessionService::with_spawner(
            DeviceId::from_array([20; 32]),
            ResourceLimits::default(),
            |_size, _cwd| panic!("spawner must not run"),
        );
        let principal = service.local_principal(AttachmentId::from_array([4; 16]));
        let lease = service
            .issue_operation_lease(principal)
            .expect("issued replay lease");
        let operation_id = OperationId { lease, sequence: 1 };
        let fingerprint = OperationFingerprint::Close {
            session_id: SessionId::from_array([4; 16]),
        };
        let (entered, observed) = mpsc::sync_channel(1);
        let (release, released) = mpsc::sync_channel(1);
        let executor_service = service.clone();
        let executor_fingerprint = fingerprint.clone();
        let executor = thread::spawn(move || {
            executor_service.execute_replayed(principal, operation_id, executor_fingerprint, || {
                entered.send(()).expect("panic observer");
                released.recv().expect("panic release");
                panic!("deterministic mutation panic");
            })
        });
        observed.recv().expect("executor entered");
        let waiter_service = service.clone();
        let waiter = thread::spawn(move || {
            waiter_service.execute_replayed(principal, operation_id, fingerprint, || {
                panic!("duplicate closure must never run")
            })
        });
        release.send(()).expect("release panic");
        for result in [
            executor.join().expect("executor thread joins"),
            waiter.join().expect("waiter thread joins"),
        ] {
            assert_eq!(
                result.expect_err("panic is reported as unknown").kind(),
                DomainErrorKind::OperationOutcomeUnknown
            );
        }
    }

    #[test]
    fn operation_completion_recovers_poison_and_wakes_with_a_terminal_result() {
        let cell = Arc::new(OperationCell::new(OperationFingerprint::Close {
            session_id: SessionId::from_array([44; 16]),
        }));
        let poisoned = Arc::clone(&cell);
        assert!(
            thread::spawn(move || {
                let _result = poisoned.result.lock().expect("operation result lock");
                panic!("inject operation-result poison");
            })
            .join()
            .is_err()
        );

        cell.complete(Err(outcome_unknown()));
        assert_eq!(
            cell.wait()
                .expect_err("poison recovery retains terminal unknown result")
                .kind(),
            DomainErrorKind::OperationOutcomeUnknown
        );
    }

    #[cfg(unix)]
    #[test]
    fn actor_mailbox_is_exactly_bounded_and_recovers_after_the_worker_drains() {
        let temporary = tempfile::tempdir().expect("temporary actor-mailbox fixture");
        let working_directory = temporary.path().to_path_buf();
        let sleep = [Path::new("/bin/sleep"), Path::new("/usr/bin/sleep")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("sleep fixture")
            .to_path_buf();
        let service = SessionService::with_spawner(
            DeviceId::from_array([0x33; DeviceId::LENGTH]),
            ResourceLimits::default(),
            move |size, requested| {
                let cwd = requested.unwrap_or(&working_directory).to_path_buf();
                let session = PtyHost::new()
                    .spawn(
                        ExplicitPtyCommand::new(&sleep, &cwd).arg("30"),
                        PtySize::new(size.rows, size.columns),
                    )
                    .map_err(map_pty_error)?;
                Ok((session, cwd))
            },
        );
        let principal =
            service.local_principal(AttachmentId::from_array([0x34; AttachmentId::LENGTH]));
        let lease = service
            .issue_operation_lease(principal)
            .expect("mailbox fixture lease");
        let summary = service
            .create(
                principal,
                OperationId { lease, sequence: 1 },
                SessionName::new("mailbox-bound").expect("mailbox fixture name"),
                None,
                None,
            )
            .expect("mailbox fixture creates");
        let actor = service
            .resolve(&SessionSelector::Id(summary.session_id))
            .expect("mailbox fixture actor resolves");

        let (entered, entered_receiver) = mpsc::sync_channel(1);
        let (release, release_receiver) = mpsc::sync_channel(1);
        let blocked_actor = Arc::clone(&actor);
        let blocker = thread::spawn(move || {
            blocked_actor.block_pty_effect_for_test(
                Instant::now() + Duration::from_secs(5),
                entered,
                release_receiver,
                Arc::new(AtomicUsize::new(0)),
            )
        });
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("actor entered the deterministic blocking boundary");

        let (queued_entered, queued_entered_receiver) = mpsc::sync_channel(1);
        let (queued_release, queued_release_receiver) = mpsc::sync_channel(1);
        let queued_executions = Arc::new(AtomicUsize::new(0));
        let queued_deadline = Instant::now() + Duration::from_secs(5);
        let queued_waiter = actor
            .enqueue_command(queued_deadline, {
                let queued_executions = Arc::clone(&queued_executions);
                move |meta, reply| SessionCommand::BlockPtyEffect {
                    meta,
                    entered: queued_entered,
                    release: queued_release_receiver,
                    executions: queued_executions,
                    reply,
                }
            })
            .expect("the first production mailbox slot admits a deterministic barrier");
        for _ in 1..SESSION_COMMAND_CAPACITY {
            assert!(
                actor.commands.try_send(SessionCommand::Wake).is_ok(),
                "every production mailbox slot is admitted while the worker is blocked",
            );
        }
        let Err(saturated) = actor.enqueue_command(Instant::now(), |meta, reply| {
            SessionCommand::CountRemoteAttachments {
                meta,
                device_id: DeviceId::from_array([0x35; DeviceId::LENGTH]),
                reply,
            }
        }) else {
            panic!("the next actor command must be backpressured at the exact bound");
        };
        assert_eq!(saturated.kind(), DomainErrorKind::DeadlineExceeded);

        release.send(()).expect("release blocked actor worker");
        blocker
            .join()
            .expect("mailbox blocker thread joins")
            .expect("mailbox blocker completes");

        queued_entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("the worker receives exactly the first queued command");
        assert_eq!(queued_executions.load(Ordering::Acquire), 1);

        let recovered_deadline = Instant::now() + Duration::from_secs(5);
        let recovered = actor
            .enqueue_command(recovered_deadline, |meta, reply| {
                SessionCommand::CountRemoteAttachments {
                    meta,
                    device_id: DeviceId::from_array([0x36; DeviceId::LENGTH]),
                    reply,
                }
            })
            .expect("one receive recovers exactly one production mailbox slot");
        let Err(full_again) = actor.enqueue_command(Instant::now(), |meta, reply| {
            SessionCommand::CountRemoteAttachments {
                meta,
                device_id: DeviceId::from_array([0x37; DeviceId::LENGTH]),
                reply,
            }
        }) else {
            panic!("the recovered slot must restore the exact mailbox bound");
        };
        assert_eq!(full_again.kind(), DomainErrorKind::DeadlineExceeded);

        queued_release
            .send(())
            .expect("release the first queued actor command");
        queued_waiter
            .wait(queued_deadline)
            .expect("the queued barrier completes after release");
        assert_eq!(
            recovered
                .wait(recovered_deadline)
                .expect("the recovered mailbox command completes"),
            0,
        );

        service.shutdown().expect("mailbox fixture shuts down");
    }

    #[cfg(unix)]
    #[test]
    fn blocked_session_effect_isolated_from_status_deadlines_and_another_session() {
        let temporary = tempfile::tempdir().expect("temporary session-effect fixture");
        let working_directory = temporary.path().to_path_buf();
        let shell = [Path::new("/bin/sh"), Path::new("/usr/bin/sh")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX shell fixture")
            .to_path_buf();
        let service = SessionService::with_spawner(
            DeviceId::from_array([61; 32]),
            ResourceLimits::default(),
            move |size, requested| {
                let cwd = requested.unwrap_or(&working_directory).to_path_buf();
                let session = PtyHost::new()
                    .spawn(
                        ExplicitPtyCommand::new(&shell, &cwd).arg("-i"),
                        PtySize::new(size.rows, size.columns),
                    )
                    .map_err(map_pty_error)?;
                Ok((session, cwd))
            },
        );
        let principal = service.local_principal(AttachmentId::from_array([7; 16]));
        let operation_lease = service
            .issue_operation_lease(principal)
            .expect("fixture operation lease");
        let create = |sequence, name: &str| {
            service
                .create(
                    principal,
                    OperationId {
                        lease: operation_lease,
                        sequence,
                    },
                    SessionName::new(name).expect("test session name"),
                    None,
                    None,
                )
                .expect("test session creates")
        };
        let first = create(1, "blocked-a");
        let second = create(2, "responsive-b");
        let first_actor = service
            .resolve(&SessionSelector::Id(first.session_id))
            .expect("first actor resolves");
        let second_actor = service
            .resolve(&SessionSelector::Id(second.session_id))
            .expect("second actor resolves");

        let first_executions = Arc::new(AtomicUsize::new(0));
        let (first_entered, entered) = mpsc::sync_channel(1);
        let (release_first, first_release) = mpsc::sync_channel(1);
        let (first_result, result) = mpsc::sync_channel(1);
        let blocked_actor = Arc::clone(&first_actor);
        let blocked_executions = Arc::clone(&first_executions);
        let blocked_thread = thread::spawn(move || {
            let result = blocked_actor.block_pty_effect_for_test(
                Instant::now() + Duration::from_secs(3),
                first_entered,
                first_release,
                blocked_executions,
            );
            let _ = first_result.send(result);
        });
        entered
            .recv_timeout(Duration::from_secs(1))
            .expect("session A entered the blocking PTY-effect boundary");
        assert_eq!(first_executions.load(Ordering::Acquire), 1);

        let (listed, listed_result) = mpsc::sync_channel(1);
        let list_service = service.clone();
        let list_thread = thread::spawn(move || {
            let _ = listed.send(list_service.list());
        });
        assert_eq!(
            listed_result
                .recv_timeout(Duration::from_millis(250))
                .expect("status remains responsive")
                .expect("status succeeds")
                .len(),
            2
        );
        list_thread.join().expect("status thread joins");

        let (second_entered, second_started) = mpsc::sync_channel(1);
        let (release_second, second_release) = mpsc::sync_channel(1);
        release_second.send(()).expect("pre-release session B");
        second_actor
            .block_pty_effect_for_test(
                Instant::now() + Duration::from_millis(250),
                second_entered,
                second_release,
                Arc::new(AtomicUsize::new(0)),
            )
            .expect("session B executes while session A is blocked");
        second_started
            .recv_timeout(Duration::from_millis(250))
            .expect("session B entered its effect boundary");

        let expired_executions = Arc::new(AtomicUsize::new(0));
        let (expired_entered, expired_started) = mpsc::sync_channel(1);
        let (expired_release, expired_receiver) = mpsc::sync_channel(1);
        expired_release
            .send(())
            .expect("pre-release expired command");
        let expired = first_actor
            .block_pty_effect_for_test(
                Instant::now() + Duration::from_millis(30),
                expired_entered,
                expired_receiver,
                Arc::clone(&expired_executions),
            )
            .expect_err("queued session-A command expires independently");
        assert_eq!(expired.kind(), DomainErrorKind::DeadlineExceeded);

        release_first.send(()).expect("release session A");
        result
            .recv_timeout(Duration::from_secs(1))
            .expect("blocking session-A effect finishes")
            .expect("blocking session-A effect succeeds");
        blocked_thread.join().expect("blocked effect thread joins");

        let (fence_entered, fence_started) = mpsc::sync_channel(1);
        let (release_fence, fence_release) = mpsc::sync_channel(1);
        release_fence.send(()).expect("pre-release fence");
        first_actor
            .block_pty_effect_for_test(
                Instant::now() + Duration::from_secs(1),
                fence_entered,
                fence_release,
                Arc::new(AtomicUsize::new(0)),
            )
            .expect("post-expiration fence executes");
        fence_started
            .recv_timeout(Duration::from_millis(250))
            .expect("post-expiration fence entered");
        assert_eq!(expired_executions.load(Ordering::Acquire), 0);
        assert!(expired_started.try_recv().is_err());

        service.shutdown().expect("fixture sessions shut down");
    }

    #[cfg(unix)]
    #[test]
    fn blocked_session_does_not_prevent_remote_detach_on_another_session() {
        let temporary = tempfile::tempdir().expect("temporary remote-detach fixture");
        let working_directory = temporary.path().to_path_buf();
        let shell = [Path::new("/bin/sh"), Path::new("/usr/bin/sh")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX shell fixture")
            .to_path_buf();
        let service = SessionService::with_spawner(
            DeviceId::from_array([63; 32]),
            ResourceLimits::default(),
            move |size, requested| {
                let cwd = requested.unwrap_or(&working_directory).to_path_buf();
                let session = PtyHost::new()
                    .spawn(
                        ExplicitPtyCommand::new(&shell, &cwd).arg("-i"),
                        PtySize::new(size.rows, size.columns),
                    )
                    .map_err(map_pty_error)?;
                Ok((session, cwd))
            },
        );
        let principal = service.local_principal(AttachmentId::from_array([9; 16]));
        let operation_lease = service
            .issue_operation_lease(principal)
            .expect("fixture operation lease");
        let create = |sequence, name: &str| {
            service
                .create(
                    principal,
                    OperationId {
                        lease: operation_lease,
                        sequence,
                    },
                    SessionName::new(name).expect("test session name"),
                    None,
                    None,
                )
                .expect("test session creates")
        };
        let blocked = create(1, "blocked-a");
        let responsive = create(2, "responsive-b");
        let blocked_actor = service
            .resolve(&SessionSelector::Id(blocked.session_id))
            .expect("blocked actor resolves");

        // One remote endpoint becomes the controller of both sessions. The
        // prepared handles stay alive so their actor attachments are not reaped
        // as detached before the detach under test.
        let remote = AttachmentPrincipal::RemoteEndpoint {
            device_id: DeviceId::from_array([0x63; 32]),
            auth_generation: 1,
        };
        let _blocked_attach = service
            .prepare_attach(
                remote,
                Some(SessionSelector::Id(blocked.session_id)),
                false,
                false,
                None,
            )
            .expect("blocked session remote attach succeeds");
        let _responsive_attach = service
            .prepare_attach(
                remote,
                Some(SessionSelector::Id(responsive.session_id)),
                false,
                false,
                None,
            )
            .expect("responsive session remote attach succeeds");

        // Deterministically block session A inside its PTY-effect boundary.
        let (entered, entered_rx) = mpsc::sync_channel(1);
        let (release, release_rx) = mpsc::sync_channel(1);
        let blocked_actor_thread = Arc::clone(&blocked_actor);
        let block_thread = thread::spawn(move || {
            blocked_actor_thread.block_pty_effect_for_test(
                Instant::now() + Duration::from_secs(3),
                entered,
                release_rx,
                Arc::new(AtomicUsize::new(0)),
            )
        });
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("session A entered its blocking boundary");

        // Detach the remote endpoint in a background thread. Phase one admits a
        // command to every actor before phase two waits, so the responsive
        // session must be released even while A is still blocked. A test-only
        // observer fires inside the responsive actor after it has processed its
        // detach command, giving a deterministic barrier instead of a poll.
        let responsive_id = responsive.session_id;
        let (processed_tx, processed_rx) = mpsc::sync_channel(1);
        let detach_service = service.clone();
        let (impact_tx, impact_rx) = mpsc::sync_channel(1);
        let detach_thread = thread::spawn(move || {
            let impact = detach_service.detach_remote_principal_until_observed(
                DeviceId::from_array([0x63; 32]),
                Instant::now() + Duration::from_secs(5),
                move |session_id| (session_id == responsive_id).then(|| processed_tx.clone()),
            );
            let _ = impact_tx.send(impact);
        });

        processed_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("responsive actor processed its detach while session A stayed blocked");

        // The observer fires after the actor refreshes its cached summary, so a
        // single synchronous read (no polling) is deterministic evidence.
        let controller_of = |session_id: SessionId| {
            service
                .list()
                .expect("list succeeds")
                .into_iter()
                .any(|summary| summary.session_id == session_id && summary.has_controller)
        };
        assert!(
            !controller_of(responsive_id),
            "responsive session remote controller was not released while session A stayed blocked"
        );
        assert!(
            controller_of(blocked.session_id),
            "blocked session controller must remain attached until its actor is released"
        );

        release.send(()).expect("release session A");
        let impact = impact_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("remote detach completes after session A releases")
            .expect("remote detach succeeds");
        assert_eq!(impact.sessions_affected, 2);
        assert_eq!(impact.attachments_removed, 2);
        assert_eq!(impact.controllers_released, 2);
        block_thread
            .join()
            .expect("blocked effect thread joins")
            .expect("blocked effect succeeds");
        detach_thread.join().expect("detach thread joins");

        service.shutdown().expect("fixture sessions shut down");
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_is_concurrent_bounded_and_truthful_until_owned_children_are_reaped() {
        let temporary = tempfile::tempdir().expect("temporary shutdown fixture");
        let working_directory = temporary.path().to_path_buf();
        let shell = [Path::new("/bin/sh"), Path::new("/usr/bin/sh")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX shell fixture")
            .to_path_buf();
        let spawn_index = Arc::new(AtomicUsize::new(0));
        let process_ids = Arc::new(Mutex::new(Vec::new()));
        let service = SessionService::with_spawner(
            DeviceId::from_array([62; 32]),
            ResourceLimits::default(),
            {
                let spawn_index = Arc::clone(&spawn_index);
                let process_ids = Arc::clone(&process_ids);
                move |size, requested| {
                    let cwd = requested.unwrap_or(&working_directory).to_path_buf();
                    let command = if spawn_index.fetch_add(1, Ordering::AcqRel) == 0 {
                        ExplicitPtyCommand::new(&shell, &cwd)
                            .arg("-c")
                            .arg("trap '' HUP; printf 'STUCK-READY\\r\\n'; while :; do :; done")
                    } else {
                        ExplicitPtyCommand::new(&shell, &cwd)
                            .arg("-c")
                            .arg("exec /bin/sleep 30")
                    };
                    let session = PtyHost::new()
                        .spawn(command, PtySize::new(size.rows, size.columns))
                        .map_err(map_pty_error)?;
                    process_ids
                        .lock()
                        .map_err(|_| synchronization_error("shutdown fixture process IDs"))?
                        .push(session.process_id().expect("fixture process id"));
                    Ok((session, cwd))
                }
            },
        );
        let principal = service.local_principal(AttachmentId::from_array([8; 16]));
        let operation_lease = service
            .issue_operation_lease(principal)
            .expect("fixture operation lease");
        let stuck = service
            .create(
                principal,
                OperationId {
                    lease: operation_lease,
                    sequence: 1,
                },
                SessionName::new("stuck-a").expect("valid stuck name"),
                None,
                None,
            )
            .expect("stuck fixture creates");
        let fast = service
            .create(
                principal,
                OperationId {
                    lease: operation_lease,
                    sequence: 2,
                },
                SessionName::new("fast-b").expect("valid fast name"),
                None,
                None,
            )
            .expect("fast fixture creates");
        let stuck_actor = service
            .inner
            .resolve(&SessionSelector::Id(stuck.session_id))
            .expect("stuck actor remains addressable");
        let fast_actor = service
            .inner
            .resolve(&SessionSelector::Id(fast.session_id))
            .expect("fast actor remains addressable");
        let prepared = service
            .prepare_attach(
                principal,
                Some(SessionSelector::Id(stuck.session_id)),
                false,
                false,
                None,
            )
            .expect("stuck fixture attaches");
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = prepared
                .attachment
                .sync_latest(Revision::ZERO)
                .expect("stuck fixture snapshot");
            if terminal_snapshot_contains(&snapshot, b"STUCK-READY") {
                break;
            }
            assert!(
                Instant::now() < ready_deadline,
                "HUP-resistant child never installed its handler"
            );
            thread::sleep(Duration::from_millis(5));
        }

        let process_ids = process_ids
            .lock()
            .expect("shutdown fixture process IDs")
            .clone();
        let stuck_pid = i32::try_from(process_ids[0]).expect("stuck pid fits pid_t");
        let fast_pid = i32::try_from(process_ids[1]).expect("fast pid fits pid_t");
        let started = Instant::now();
        let error = service
            .shutdown_until(Instant::now() + Duration::from_millis(20))
            .expect_err("shutdown cannot claim success while the resistant child is owned");
        assert_eq!(error.kind(), DomainErrorKind::DeadlineExceeded);
        assert!(started.elapsed() < Duration::from_millis(150));
        assert!(error.detail().contains("stuck-a"));
        assert!(process_exists(stuck_pid));
        assert_eq!(
            stuck_actor.requested_end_reason(),
            Some(SessionEndReason::DaemonStop)
        );
        assert_eq!(
            fast_actor.requested_end_reason(),
            Some(SessionEndReason::DaemonStop),
            "shutdown must request every owner before waiting for cleanup"
        );
        assert!(
            service
                .list()
                .expect("status remains available after failed shutdown")
                .iter()
                .any(|summary| summary.session_id == stuck.session_id)
        );

        let completion_deadline = Instant::now() + Duration::from_secs(3);
        while !service.list().expect("poll shutdown completion").is_empty()
            && Instant::now() < completion_deadline
        {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            service.list().expect("final shutdown status").is_empty(),
            "the original concurrent shutdown did not finish after escalation"
        );
        assert!(!process_exists(stuck_pid));
        assert!(!process_exists(fast_pid));
        assert!(
            service
                .shutdown_until(Instant::now() + Duration::from_secs(1))
                .expect("a retry observes fully released ownership")
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn publication_loss_after_spawn_closes_the_child_and_releases_the_reservation() {
        let temporary = tempfile::tempdir().expect("temporary publication-loss fixture");
        let working_directory = temporary.path().to_path_buf();
        let shell = [Path::new("/bin/sh"), Path::new("/usr/bin/sh")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX shell fixture")
            .to_path_buf();
        let (spawned, spawned_child) = mpsc::sync_channel(1);
        let (release_spawn, release) = mpsc::sync_channel(1);
        let release = Arc::new(Mutex::new(release));
        let service = SessionService::with_spawner(
            DeviceId::from_array([63; 32]),
            ResourceLimits::default(),
            {
                let release = Arc::clone(&release);
                move |size, requested| {
                    let cwd = requested.unwrap_or(&working_directory).to_path_buf();
                    let session = PtyHost::new()
                        .spawn(
                            ExplicitPtyCommand::new(&shell, &cwd)
                                .arg("-c")
                                .arg("exec /bin/sleep 30"),
                            PtySize::new(size.rows, size.columns),
                        )
                        .map_err(map_pty_error)?;
                    spawned
                        .send(session.process_id().expect("fixture process id"))
                        .map_err(|_| synchronization_error("spawn observer"))?;
                    release
                        .lock()
                        .map_err(|_| synchronization_error("spawn release"))?
                        .recv()
                        .map_err(|_| synchronization_error("spawn release"))?;
                    Ok((session, cwd))
                }
            },
        );
        let principal = service.local_principal(AttachmentId::from_array([9; 16]));
        let operation_lease = service
            .issue_operation_lease(principal)
            .expect("fixture operation lease");
        let create_service = service.clone();
        let (created, create_result) = mpsc::sync_channel(1);
        let create_thread = thread::spawn(move || {
            let result = create_service.create(
                principal,
                OperationId {
                    lease: operation_lease,
                    sequence: 1,
                },
                SessionName::new("publication-loss").expect("valid fixture name"),
                None,
                None,
            );
            let _ = created.send(result);
        });
        let process_id = i32::try_from(
            spawned_child
                .recv_timeout(Duration::from_secs(1))
                .expect("child spawned behind the provisional reservation"),
        )
        .expect("fixture pid fits pid_t");

        let stop = service
            .shutdown_until(Instant::now() + Duration::from_millis(20))
            .expect_err("provisional child ownership prevents a truthful stop");
        assert_eq!(stop.kind(), DomainErrorKind::DeadlineExceeded);
        assert!(process_exists(process_id));
        assert!(
            service
                .list()
                .expect("diagnostic list remains available")
                .is_empty()
        );

        release_spawn.send(()).expect("release spawned session");
        let error = create_result
            .recv_timeout(Duration::from_secs(2))
            .expect("cancelled create finishes cleanup")
            .expect_err("cancelled publication cannot become live");
        assert_eq!(error.kind(), DomainErrorKind::Cancelled);
        create_thread.join().expect("create thread joins");
        assert!(!process_exists(process_id));
        assert!(
            service
                .list()
                .expect("no half-published session")
                .is_empty()
        );
        assert!(
            service
                .shutdown_until(Instant::now() + Duration::from_secs(1))
                .expect("cleanup releases the provisional resource")
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn creation_actor_and_close_panics_leave_retriable_owned_state() {
        let temporary = tempfile::tempdir().expect("temporary unwind fixture");
        let working_directory = temporary.path().to_path_buf();
        let shell = [Path::new("/bin/sh"), Path::new("/usr/bin/sh")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX shell fixture")
            .to_path_buf();
        let process_ids = Arc::new(Mutex::new(Vec::new()));
        let service = SessionService::with_spawner(
            DeviceId::from_array([64; 32]),
            ResourceLimits::default(),
            {
                let process_ids = Arc::clone(&process_ids);
                move |size, requested| {
                    let cwd = requested.unwrap_or(&working_directory).to_path_buf();
                    let session = PtyHost::new()
                        .spawn(
                            ExplicitPtyCommand::new(&shell, &cwd)
                                .arg("-c")
                                .arg("exec /bin/sleep 30"),
                            PtySize::new(size.rows, size.columns),
                        )
                        .map_err(map_pty_error)?;
                    process_ids
                        .lock()
                        .map_err(|_| synchronization_error("unwind fixture process IDs"))?
                        .push(session.process_id().expect("fixture process id"));
                    Ok((session, cwd))
                }
            },
        );
        let principal = service.local_principal(AttachmentId::from_array([10; 16]));
        let lease = service
            .issue_operation_lease(principal)
            .expect("unwind fixture lease");

        service.panic_next_creation_after_spawn_for_test();
        let creation = service
            .create(
                principal,
                OperationId { lease, sequence: 1 },
                SessionName::new("creation-panic").expect("fixture name"),
                None,
                None,
            )
            .expect_err("creation panic is not hidden as success");
        assert_eq!(creation.kind(), DomainErrorKind::OperationOutcomeUnknown);
        assert!(
            service
                .list()
                .expect("registry after creation panic")
                .is_empty()
        );
        assert_eq!(service.inner.reservation_count().expect("reservations"), 0);

        let actor_summary = service
            .create(
                principal,
                OperationId { lease, sequence: 2 },
                SessionName::new("actor-panic").expect("fixture name"),
                None,
                None,
            )
            .expect("actor panic fixture creates");
        let actor = service
            .resolve(&SessionSelector::Id(actor_summary.session_id))
            .expect("actor resolves");
        actor
            .panic_worker_for_test(Instant::now() + Duration::from_secs(2))
            .expect("actor worker panic finalizes");
        actor.join_finished().expect("panicked actor thread joins");
        assert!(
            service
                .list()
                .expect("registry after actor panic")
                .is_empty()
        );
        assert_eq!(service.inner.reservation_count().expect("reservations"), 0);

        let close_summary = service
            .create(
                principal,
                OperationId { lease, sequence: 3 },
                SessionName::new("close-panic").expect("fixture name"),
                None,
                None,
            )
            .expect("close panic fixture creates");
        let close_actor = service
            .resolve(&SessionSelector::Id(close_summary.session_id))
            .expect("close actor resolves");
        close_actor.panic_next_close_for_test();
        close_actor.begin_end(SessionEndReason::ExplicitClose);
        let first_close = close_actor
            .wait_finished_until(Instant::now() + Duration::from_secs(1))
            .expect_err("close panic is surfaced");
        assert_eq!(first_close.kind(), DomainErrorKind::OperationOutcomeUnknown);
        close_actor.begin_end(SessionEndReason::ExplicitClose);
        close_actor
            .wait_finished_until(Instant::now() + Duration::from_secs(2))
            .expect("close retry releases child");
        close_actor.join_finished().expect("close actor joins");
        assert!(
            service
                .list()
                .expect("registry after close retry")
                .is_empty()
        );
        assert_eq!(service.inner.reservation_count().expect("reservations"), 0);

        let poisoned = service
            .create(
                principal,
                OperationId { lease, sequence: 4 },
                SessionName::new("summary-panic").expect("fixture name"),
                None,
                None,
            )
            .expect("summary panic fixture creates");
        service
            .create(
                principal,
                OperationId { lease, sequence: 5 },
                SessionName::new("summary-peer").expect("fixture name"),
                None,
                None,
            )
            .expect("summary peer creates");
        let poison_actor = service
            .resolve(&SessionSelector::Id(poisoned.session_id))
            .expect("poison actor resolves");
        let poison = thread::spawn(move || {
            let _cached = poison_actor.cached.lock().expect("summary cache locks");
            panic!("injected summary-cache poison");
        });
        assert!(poison.join().is_err());
        let shutdown = service
            .shutdown_until(Instant::now() + Duration::from_secs(2))
            .expect_err("typed summary failure is surfaced after closing every owner");
        assert_eq!(shutdown.kind(), DomainErrorKind::StoreUnavailable);
        assert!(
            service
                .inner
                .owned_entries()
                .expect("owned entries")
                .is_empty()
        );
        assert_eq!(service.inner.reservation_count().expect("reservations"), 0);

        let retry = service
            .create(
                principal,
                OperationId { lease, sequence: 6 },
                SessionName::new("after-summary-error").expect("fixture name"),
                None,
                None,
            )
            .expect("failed shutdown resumes mutation admission");
        service
            .close(
                principal,
                OperationId { lease, sequence: 7 },
                retry.session_id,
            )
            .expect("cleanup retry fixture closes");

        for process_id in process_ids.lock().expect("process IDs").iter().copied() {
            let process_id = i32::try_from(process_id).expect("pid fits pid_t");
            assert!(
                !process_exists(process_id),
                "child {process_id} survived unwind cleanup"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn session_id_collision_never_overwrites_an_existing_owner_reservation() {
        let temporary = tempfile::tempdir().expect("temporary ID-collision fixture");
        let service = unix_fixture_service(
            DeviceId::from_array([70; 32]),
            temporary.path().to_path_buf(),
            "exec /bin/sleep 30",
        );
        let principal = service.local_principal(AttachmentId::from_array([70; 16]));
        let lease = service
            .issue_operation_lease(principal)
            .expect("collision fixture lease");
        let first = service
            .create(
                principal,
                OperationId { lease, sequence: 1 },
                SessionName::new("collision-first").expect("first name"),
                None,
                None,
            )
            .expect("first owner creates");
        let replacement = SessionId::from_array([0x5a; 16]);
        assert_ne!(first.session_id, replacement);
        service
            .inner
            .inject_session_id_candidates([first.session_id, replacement]);
        let second = service
            .create(
                principal,
                OperationId { lease, sequence: 2 },
                SessionName::new("collision-second").expect("second name"),
                None,
                None,
            )
            .expect("collision candidate is skipped atomically");
        assert_eq!(second.session_id, replacement);
        assert_eq!(service.inner.reservation_count().expect("reservations"), 2);

        service
            .close(
                principal,
                OperationId { lease, sequence: 3 },
                first.session_id,
            )
            .expect("first owner closes");
        assert_eq!(service.inner.reservation_count().expect("reservations"), 1);
        assert!(
            service
                .resolve(&SessionSelector::Id(second.session_id))
                .is_ok()
        );
        service
            .close(
                principal,
                OperationId { lease, sequence: 4 },
                second.session_id,
            )
            .expect("second owner closes");
        assert_eq!(service.inner.reservation_count().expect("reservations"), 0);
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_joins_each_cleanup_owner_even_when_ids_conflict() {
        let temporary = tempfile::tempdir().expect("temporary cleanup-owner fixture");
        let working_directory = temporary.path().to_path_buf();
        let service = unix_fixture_service(
            DeviceId::from_array([74; 32]),
            working_directory.clone(),
            "exec /bin/sleep 30",
        );
        let principal = service.local_principal(AttachmentId::from_array([74; 16]));
        let lease = service
            .issue_operation_lease(principal)
            .expect("cleanup-owner lease");
        let first = service
            .create(
                principal,
                OperationId { lease, sequence: 1 },
                SessionName::new("collision-live").expect("live name"),
                None,
                None,
            )
            .expect("live owner creates");
        let first_actor = service
            .resolve(&SessionSelector::Id(first.session_id))
            .expect("live actor resolves");

        // Model the defensive cleanup-only state used after an impossible
        // poisoned/corrupt registration collision: it has its own actor/token
        // but the externally supplied ID conflicts with a live owner.
        let size = TerminalSize::new(24, 80);
        let model = TerminalModel::new(size, ResourceLimits::default().recent_history_rows)
            .expect("cleanup-owner model");
        let shell = [Path::new("/bin/sh"), Path::new("/usr/bin/sh")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX shell fixture");
        let session = PtyHost::new()
            .spawn(
                ExplicitPtyCommand::new(shell, &working_directory)
                    .arg("-c")
                    .arg("exec /bin/sleep 30"),
                PtySize::new(size.rows, size.columns),
            )
            .expect("cleanup-only PTY spawns");
        let driver = TerminalDriver::start(session, model, TerminalDriverConfig::default())
            .expect("cleanup-only driver starts");
        let cleanup_actor = SessionActor::start(
            first.session_id,
            size,
            driver,
            Arc::downgrade(&service.inner),
            ResourceLimits::default(),
        )
        .expect("cleanup-only actor starts");
        cleanup_actor.mark_registry_owned();
        cleanup_lock(&service.inner.state)
            .cleanup_only
            .push(SessionEntry {
                actor: Arc::clone(&cleanup_actor),
                name: SessionName::new("collision-cleanup").expect("cleanup name"),
                working_directory,
                ownership: OwnershipToken::new(),
            });

        service
            .shutdown_until(Instant::now() + Duration::from_secs(2))
            .expect("both conflicting owners shut down");
        assert!(
            cleanup_lock(&first_actor.worker).is_none(),
            "live actor handle was joined"
        );
        assert!(
            cleanup_lock(&cleanup_actor.worker).is_none(),
            "cleanup-only actor handle was joined independently"
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_publication_retains_name_until_timeout_cleanup_reaps_the_actor() {
        let temporary = tempfile::tempdir().expect("temporary cleanup-timeout fixture");
        let working_directory = temporary.path().to_path_buf();
        let shell = [Path::new("/bin/sh"), Path::new("/usr/bin/sh")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX shell fixture")
            .to_path_buf();
        let service = SessionService::with_spawner(
            DeviceId::from_array([71; 32]),
            ResourceLimits::default(),
            move |size, requested| {
                let cwd = requested.unwrap_or(&working_directory).to_path_buf();
                let session = PtyHost::new()
                    .spawn(
                        ExplicitPtyCommand::new(&shell, &cwd)
                            .arg("-c")
                            .arg("trap '' HUP; : > .cleanup-timeout-ready; while :; do :; done"),
                        PtySize::new(size.rows, size.columns),
                    )
                    .map_err(map_pty_error)?;
                let ready = cwd.join(".cleanup-timeout-ready");
                let ready_deadline = Instant::now() + Duration::from_secs(2);
                while !ready.is_file() {
                    if Instant::now() >= ready_deadline {
                        return Err(synchronization_error("cleanup-timeout child readiness"));
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                Ok((session, cwd))
            },
        );
        let principal = service.local_principal(AttachmentId::from_array([71; 16]));
        let lease = service
            .issue_operation_lease(principal)
            .expect("cleanup-timeout lease");
        let name = SessionName::new("cleanup-timeout").expect("fixture name");
        service.inner.fail_next_provisional_registration();
        let error = service
            .create_until(
                principal,
                OperationId { lease, sequence: 1 },
                name.clone(),
                None,
                None,
                Instant::now() + Duration::from_millis(20),
            )
            .expect_err("bounded creation cleanup times out");
        assert_eq!(error.kind(), DomainErrorKind::DeadlineExceeded);
        assert_eq!(service.inner.owned_entries().expect("owned actor").len(), 1);
        assert_eq!(service.inner.reservation_count().expect("reservation"), 1);
        let duplicate = service
            .create(
                principal,
                OperationId { lease, sequence: 2 },
                name.clone(),
                None,
                None,
            )
            .expect_err("starting name remains unavailable during cleanup");
        assert_eq!(duplicate.kind(), DomainErrorKind::SessionAlreadyExists);

        let deadline = Instant::now() + Duration::from_secs(3);
        while service.inner.reservation_count().expect("poll reservation") != 0 {
            assert!(
                Instant::now() < deadline,
                "cleanup never released ownership"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let replacement = service
            .create(
                principal,
                OperationId { lease, sequence: 3 },
                name,
                None,
                None,
            )
            .expect("same name is admitted after actual cleanup");
        service
            .close(
                principal,
                OperationId { lease, sequence: 4 },
                replacement.session_id,
            )
            .expect("replacement closes");
    }

    #[cfg(unix)]
    #[test]
    fn failed_publication_cleanup_error_keeps_a_retriable_provisional_owner() {
        let temporary = tempfile::tempdir().expect("temporary cleanup-error fixture");
        let service = unix_fixture_service(
            DeviceId::from_array([72; 32]),
            temporary.path().to_path_buf(),
            "exec /bin/sleep 30",
        );
        let principal = service.local_principal(AttachmentId::from_array([72; 16]));
        let lease = service
            .issue_operation_lease(principal)
            .expect("cleanup-error lease");
        let name = SessionName::new("cleanup-error").expect("fixture name");
        service.inner.fail_next_provisional_registration();
        service.panic_next_creation_cleanup_for_test();
        let error = service
            .create(
                principal,
                OperationId { lease, sequence: 1 },
                name.clone(),
                None,
                None,
            )
            .expect_err("cleanup panic is surfaced");
        assert_eq!(error.kind(), DomainErrorKind::OperationOutcomeUnknown);
        let owned = service.inner.owned_entries().expect("retriable owner");
        assert_eq!(owned.len(), 1);
        assert_eq!(service.inner.reservation_count().expect("reservation"), 1);
        let duplicate = service
            .create(
                principal,
                OperationId { lease, sequence: 2 },
                name.clone(),
                None,
                None,
            )
            .expect_err("failed cleanup retains its name");
        assert_eq!(duplicate.kind(), DomainErrorKind::SessionAlreadyExists);

        owned[0].actor.begin_end(SessionEndReason::DriverFailure);
        owned[0]
            .actor
            .wait_finished_until(Instant::now() + Duration::from_secs(2))
            .expect("explicit cleanup retry reaps actor");
        owned[0].actor.join_finished().expect("cleanup actor joins");
        assert_eq!(service.inner.reservation_count().expect("reservation"), 0);
        let replacement = service
            .create(
                principal,
                OperationId { lease, sequence: 3 },
                name,
                None,
                None,
            )
            .expect("same name is admitted only after retry cleanup");
        service
            .close(
                principal,
                OperationId { lease, sequence: 4 },
                replacement.session_id,
            )
            .expect("replacement closes");
    }

    #[cfg(unix)]
    #[test]
    fn poisoned_registry_name_and_resource_locks_reclaim_only_matching_owner() {
        let temporary = tempfile::tempdir().expect("temporary poison fixture");
        let service = unix_fixture_service(
            DeviceId::from_array([73; 32]),
            temporary.path().to_path_buf(),
            "exec /bin/sleep 30",
        );
        let principal = service.local_principal(AttachmentId::from_array([73; 16]));
        let lease = service
            .issue_operation_lease(principal)
            .expect("poison fixture lease");
        let first = service
            .create(
                principal,
                OperationId { lease, sequence: 1 },
                SessionName::new("poison-first").expect("first name"),
                None,
                None,
            )
            .expect("first creates");
        let second = service
            .create(
                principal,
                OperationId { lease, sequence: 2 },
                SessionName::new("poison-second").expect("second name"),
                None,
                None,
            )
            .expect("second creates");
        let first_actor = service
            .resolve(&SessionSelector::Id(first.session_id))
            .expect("first actor");

        let registry = Arc::clone(&service.inner);
        assert!(
            thread::spawn(move || {
                let _state = registry.state.lock().expect("registry/name lock");
                panic!("inject registry/name ownership poison");
            })
            .join()
            .is_err()
        );
        assert_eq!(
            service
                .list()
                .expect_err("ordinary API surfaces registry poison")
                .kind(),
            DomainErrorKind::StoreUnavailable
        );
        let registry = Arc::clone(&service.inner);
        assert!(
            thread::spawn(move || {
                let _reservations = registry.reservations.lock().expect("reservation lock");
                panic!("inject reservation ownership poison");
            })
            .join()
            .is_err()
        );

        first_actor.begin_end(SessionEndReason::ExplicitClose);
        first_actor
            .wait_finished_until(Instant::now() + Duration::from_secs(2))
            .expect("poison-aware finalizer closes first");
        first_actor.join_finished().expect("first actor joins");
        assert_eq!(service.inner.reservation_count().expect("reservations"), 1);
        assert!(
            service
                .resolve(&SessionSelector::Id(first.session_id))
                .is_err()
        );
        assert!(
            service
                .resolve(&SessionSelector::Id(second.session_id))
                .is_ok()
        );
        assert_eq!(
            service.list().expect("unrelated owner remains")[0].name,
            SessionName::new("poison-second").expect("second name")
        );
        service
            .close(
                principal,
                OperationId { lease, sequence: 3 },
                second.session_id,
            )
            .expect("unrelated owner closes normally");
    }

    #[cfg(unix)]
    #[test]
    fn reserve_complete_and_shutdown_share_one_non_deadlocking_lock_order() {
        let temporary = tempfile::tempdir().expect("temporary lock-order fixture");
        let cwd = temporary.path().to_path_buf();
        let shell = [Path::new("/bin/sh"), Path::new("/usr/bin/sh")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX shell fixture")
            .to_path_buf();
        let spawn_count = Arc::new(AtomicUsize::new(0));
        let (reserved, observed_reserved) = mpsc::sync_channel(1);
        let (release, released) = mpsc::sync_channel(1);
        let released = Arc::new(Mutex::new(released));
        let service = SessionService::with_spawner(
            DeviceId::from_array([74; 32]),
            ResourceLimits::default(),
            {
                let spawn_count = Arc::clone(&spawn_count);
                let released = Arc::clone(&released);
                move |size, requested| {
                    let actual = requested.unwrap_or(&cwd).to_path_buf();
                    if spawn_count.fetch_add(1, Ordering::AcqRel) == 1 {
                        reserved
                            .send(())
                            .map_err(|_| synchronization_error("reserve observer"))?;
                        released
                            .lock()
                            .map_err(|_| synchronization_error("reserve release"))?
                            .recv()
                            .map_err(|_| synchronization_error("reserve release"))?;
                    }
                    let session = PtyHost::new()
                        .spawn(
                            ExplicitPtyCommand::new(&shell, &actual)
                                .arg("-c")
                                .arg("exec /bin/sleep 30"),
                            PtySize::new(size.rows, size.columns),
                        )
                        .map_err(map_pty_error)?;
                    Ok((session, actual))
                }
            },
        );
        let principal = service.local_principal(AttachmentId::from_array([74; 16]));
        let lease = service
            .issue_operation_lease(principal)
            .expect("lock-order lease");
        let first = service
            .create(
                principal,
                OperationId { lease, sequence: 1 },
                SessionName::new("lock-live").expect("live name"),
                None,
                None,
            )
            .expect("live fixture creates");

        let create_service = service.clone();
        let (create_done, create_result) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let result = create_service.create(
                principal,
                OperationId { lease, sequence: 2 },
                SessionName::new("lock-starting").expect("starting name"),
                None,
                None,
            );
            let _ = create_done.send(result);
        });
        observed_reserved
            .recv_timeout(Duration::from_secs(1))
            .expect("second creation owns its reservation");

        let close_service = service.clone();
        let (close_done, close_result) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let result = close_service.close(
                principal,
                OperationId { lease, sequence: 3 },
                first.session_id,
            );
            let _ = close_done.send(result);
        });
        let shutdown_service = service.clone();
        let (shutdown_done, shutdown_result) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let result = shutdown_service.shutdown_until(Instant::now() + Duration::from_secs(3));
            let _ = shutdown_done.send(result);
        });
        release.send(()).expect("release reserved creation");

        let create = create_result
            .recv_timeout(Duration::from_secs(3))
            .expect("reserve path never deadlocks");
        assert!(
            create.is_ok()
                || create
                    .as_ref()
                    .is_err_and(|error| error.kind() == DomainErrorKind::Cancelled)
        );
        let close = close_result
            .recv_timeout(Duration::from_secs(3))
            .expect("completion path never deadlocks");
        assert!(
            close.is_ok()
                || close
                    .as_ref()
                    .is_err_and(|error| error.kind() == DomainErrorKind::SessionNotFound)
        );
        shutdown_result
            .recv_timeout(Duration::from_secs(3))
            .expect("shutdown path never deadlocks")
            .expect("shutdown releases all owners");
        assert_eq!(service.inner.reservation_count().expect("reservations"), 0);
    }

    #[cfg(unix)]
    async fn wait_for_attachment_text(prepared: &PreparedAttachment, expected: &[u8]) -> Revision {
        if terminal_snapshot_contains(&prepared.snapshot, expected) {
            return prepared.snapshot.revision;
        }
        let mut revisions = prepared
            .attachment
            .revision_watch()
            .expect("fixture revision watermark");
        let mut baseline = prepared.snapshot.revision;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                while *revisions.borrow_and_update() <= baseline {
                    revisions
                        .changed()
                        .await
                        .expect("driver revision stays open");
                }
                let snapshot = prepared
                    .attachment
                    .sync_latest(baseline)
                    .expect("force one deterministic fixture snapshot");
                baseline = snapshot.revision;
                let found = terminal_snapshot_contains(&snapshot, expected);
                assert!(
                    prepared
                        .attachment
                        .snapshot_applied(snapshot.revision)
                        .expect("acknowledge deterministic fixture snapshot")
                        .is_none()
                );
                if found {
                    return baseline;
                }
            }
        })
        .await
        .expect("fixture output synchronization marker reached the authoritative terminal")
    }

    #[cfg(unix)]
    fn terminal_snapshot_contains(snapshot: &TerminalSurfaceSnapshot, expected: &[u8]) -> bool {
        let Ok(expected) = std::str::from_utf8(expected) else {
            return false;
        };
        snapshot.surface.rows.iter().any(|row| {
            row.cells
                .iter()
                .map(|cell| cell.contents.as_str())
                .collect::<String>()
                .contains(expected)
        })
    }

    #[cfg(unix)]
    fn terminal_delta_contains(delta: &TerminalSurfaceDelta, expected: &str) -> bool {
        delta.row_patches.iter().any(|patch| {
            patch
                .replacement
                .cells
                .iter()
                .map(|cell| cell.contents.as_str())
                .collect::<String>()
                .contains(expected)
        })
    }

    #[cfg(unix)]
    fn unix_fixture_service(
        device_id: DeviceId,
        working_directory: PathBuf,
        script: &'static str,
    ) -> SessionService {
        let shell = [Path::new("/bin/sh"), Path::new("/usr/bin/sh")]
            .into_iter()
            .find(|path| path.is_file())
            .expect("POSIX shell fixture")
            .to_path_buf();
        SessionService::with_spawner(
            device_id,
            ResourceLimits::default(),
            move |size, requested| {
                let cwd = requested.unwrap_or(&working_directory).to_path_buf();
                let session = PtyHost::new()
                    .spawn(
                        ExplicitPtyCommand::new(&shell, &cwd).arg("-c").arg(script),
                        PtySize::new(size.rows, size.columns),
                    )
                    .map_err(map_pty_error)?;
                Ok((session, cwd))
            },
        )
    }

    #[cfg(unix)]
    fn process_exists(process_id: i32) -> bool {
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(process_id), None).is_ok()
    }
}
