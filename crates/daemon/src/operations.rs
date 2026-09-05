//! Daemon-aware command backend shared by the thin CLI.

use std::fmt;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
#[cfg(unix)]
use std::sync::Arc;
use std::time::{Duration, Instant};

use zterm_core::terminal::TerminalSize;
use zterm_core::{
    AuthGeneration, AuthorizationStatus, DeviceAlias, DeviceId, DeviceSummary, DomainErrorKind,
    Revision, SessionId, SessionName, SessionSelector,
};
use zterm_platform::user_state::UserPaths;

use crate::bootstrap::BootstrapResult;
#[cfg(unix)]
use crate::bootstrap::bootstrap_with_lock_held;
use crate::client::{LocalClient, LocalDeviceClient};
#[cfg(unix)]
use crate::client::{LocalPairingClient, RemoteDaemonRestarter, SessionClient};
use crate::config::ValidatedConfig;
use crate::device_directory::ResolvedSessionTarget;
use crate::error::DaemonError;
#[cfg(unix)]
use crate::lifecycle::acquire_lifecycle_lock;
use crate::lifecycle::{DaemonLauncher, probe_readiness};
use crate::pairing::PairTicketText;
use crate::service::{DaemonReadiness, DaemonStatus, SessionImpact};

const MAX_LOG_LINES: usize = 1_000;
const MAX_LOG_BYTES: u64 = 1024 * 1024;
const IDENTITY_RESET_STOP_TIMEOUT: Duration = Duration::from_secs(5);

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

pub use crate::client::view::{
    PreparedTerminalView, TerminalViewCommandWriter, TerminalViewConnectionPath,
    TerminalViewConnectionStatus, TerminalViewDelta, TerminalViewEndReason, TerminalViewEnded,
    TerminalViewEvent, TerminalViewEventReader, TerminalViewHistoryWindow, TerminalViewIo,
    TerminalViewRoute, TerminalViewSnapshot, TerminalViewTarget, TerminalViewTransportState,
};

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
    /// Active Sessions which would be ended by a approved reset.
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
    /// Sessions ended only after candidate verification and explicit approval.
    pub ended_session_names: Vec<String>,
    /// Whether the installed daemon reached local readiness (false before setup).
    pub daemon_started: bool,
}

/// Actual update transaction boundaries, rendered by the invoking frontend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateStage {
    /// Candidate download and authentication are beginning.
    Preparing,
    /// All candidate authentication checks completed.
    Verified,
    /// Stopping the current daemon after any required approval.
    Stopping,
    /// Replacing the installed executable.
    Activating,
    /// Starting the installed daemon and checking local readiness.
    Starting,
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

fn require_interruption_approval(
    impact: &SessionImpact,
    approved: &mut bool,
    confirm: &mut impl FnMut(&SessionImpact) -> Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    if !*approved && (impact.interruption_required || impact.active_session_count > 0) {
        confirm(impact)?;
        *approved = true;
    }
    Ok(())
}

fn confirmation_required(_: &SessionImpact) -> Result<(), DaemonError> {
    Err(DaemonError::new(
        DomainErrorKind::Cancelled,
        "Running sessions require confirmation. Run again with -y to continue without prompting.",
    ))
}

/// The post-commit policy consumes authenticated release identity, never the
/// version of the old updater still executing in this process.
#[cfg(unix)]
async fn finish_update_startup<
    F: std::future::Future<Output = Result<DaemonReadiness, DaemonError>>,
>(
    observed: Result<ObservedState, DaemonError>,
    installed: &zterm_core::release::ReleaseManifest,
    progress: &mut impl FnMut(UpdateStage),
    ensure: impl FnOnce() -> F,
) -> Result<bool, DaemonError> {
    let result = async {
        match observed? {
            ObservedState::NotConfigured => Ok(false),
            ObservedState::Running(_) | ObservedState::ConfiguredStopped(_) => {
                progress(UpdateStage::Starting);
                let ready = ensure().await?;
                if ready.version != installed.version
                    || ready.protocol.wire_major != installed.wire_major
                    || ready.protocol.state_schema != installed.state_schema
                {
                    return Err(DaemonError::new(
                        DomainErrorKind::UpdateRejected,
                        "The running daemon does not match the installed release.",
                    ));
                }
                Ok(true)
            }
        }
    }
    .await;
    result.map_err(|error| DaemonError::new(error.kind(), format!(
        "Updated zterm to {}, but the daemon could not start: {}. Run zterm daemon restart to try again.",
        installed.version, error)))
}

fn require_identity_reset_session_force(
    preflight: &IdentityResetPreflight,
    force: bool,
) -> Result<(), DaemonError> {
    if preflight.active_session_count() > 0 && !force {
        return Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            format!(
                "{} active session(s) would be interrupted; run again with -y",
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

#[cfg(unix)]
#[derive(Clone)]
struct RuntimeDaemonRestarter {
    paths: UserPaths,
    launcher: DaemonLauncher,
}

#[cfg(unix)]
impl RemoteDaemonRestarter for RuntimeDaemonRestarter {
    fn ensure_running<'a>(
        &'a self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DaemonError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.launcher.ensure(&self.paths).await?;
            Ok(())
        })
    }
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
        self.update_with_callbacks(exact_tag, force, confirmation_required, |_| {})
            .await
    }

    /// Updates once, asking the frontend only when live ownership needs interruption.
    pub async fn update_with_callbacks(
        &self,
        exact_tag: Option<&str>,
        mut approved: bool,
        mut confirm: impl FnMut(&SessionImpact) -> Result<(), DaemonError>,
        mut progress: impl FnMut(UpdateStage),
    ) -> Result<UpdateResult, DaemonError> {
        #[cfg(unix)]
        {
            let executable = self.launcher.executable();
            crate::distribution::validate_managed_executable(executable, self.paths.uid())?;
            let selection = crate::distribution::ReleaseSelection::parse(exact_tag)?;
            progress(UpdateStage::Preparing);
            let prepared = crate::distribution::prepare_update(selection).await?;
            progress(UpdateStage::Verified);

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
            require_interruption_approval(&impact, &mut approved, &mut confirm)?;
            let ended = if daemon_running {
                progress(UpdateStage::Stopping);
                self.stop_with_confirmation(approved, &mut confirm)
                    .await?
                    .map_or_else(Vec::new, |impact| impact.active_session_names)
            } else {
                Vec::new()
            };

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

            progress(UpdateStage::Activating);
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

            let daemon_started = finish_update_startup(
                self.observe().await,
                prepared.manifest(),
                &mut progress,
                || self.ensure(),
            )
            .await?;
            Ok(UpdateResult {
                previous_version: zterm_core::BuildIdentity::current().version.to_owned(),
                installed_version: prepared.version().to_owned(),
                ended_session_names: ended,
                daemon_started,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (exact_tag, approved, confirm, progress);
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
            let view_target = if let Some(device_id) = target.device_id() {
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
                TerminalViewTarget {
                    display_name: alias.as_str().to_owned(),
                    route: TerminalViewRoute::Remote,
                }
            } else {
                let status = LocalClient::new(self.paths.socket()).status().await?;
                TerminalViewTarget {
                    display_name: status.device_name,
                    route: TerminalViewRoute::Local,
                }
            };
            let mut client = SessionClient::connect_resolved(
                self.paths.socket(),
                target,
                selector,
                create_main,
                takeover,
                viewport,
            )
            .await?;
            if target.device_id().is_some() {
                client.set_remote_daemon_restarter(Arc::new(RuntimeDaemonRestarter {
                    paths: self.paths.clone(),
                    launcher: self.launcher.clone(),
                }));
            }
            PreparedTerminalView::new(client, takeover, view_target)
        }
        #[cfg(not(unix))]
        {
            let _ = (target, selector, create_main, takeover, viewport);
            Err(unsupported_command_platform())
        }
    }

    /// Stops the daemon if running; without approval live work is preserved.
    pub async fn stop(&self, approved: bool) -> Result<Option<SessionImpact>, DaemonError> {
        self.stop_with_confirmation(approved, confirmation_required)
            .await
    }

    /// Keeps an admission race in the same invocation's confirmation flow.
    pub async fn stop_with_confirmation(
        &self,
        mut approved: bool,
        mut confirm: impl FnMut(&SessionImpact) -> Result<(), DaemonError>,
    ) -> Result<Option<SessionImpact>, DaemonError> {
        let client = LocalClient::new(self.paths.socket());
        let mut impact = match client.update_preflight().await {
            Ok(impact) => impact,
            Err(error) if error.kind() == DomainErrorKind::DaemonStopped => return Ok(None),
            Err(error) => return Err(error),
        };
        loop {
            require_interruption_approval(&impact, &mut approved, &mut confirm)?;
            impact = match client.stop(approved).await {
                Ok(impact) => impact,
                Err(error) if error.kind() == DomainErrorKind::DaemonStopped => return Ok(None),
                Err(error) => return Err(error),
            };
            if impact.stopping {
                #[cfg(unix)]
                wait_until_stopped(&self.paths).await?;
                return Ok(Some(impact));
            }
            if approved {
                return Err(DaemonError::new(
                    DomainErrorKind::Cancelled,
                    "The daemon did not accept the approved stop request.",
                ));
            }
            // The server found work admitted after the observation. No shutdown
            // began; display that impact before granting interruption authority.
        }
    }

    /// Stops when needed, then explicitly ensures one configured daemon.
    pub async fn restart(&self, approved: bool) -> Result<DaemonReadiness, DaemonError> {
        self.restart_with_confirmation(approved, confirmation_required)
            .await
    }

    /// Restarts with frontend-owned conditional confirmation.
    pub async fn restart_with_confirmation(
        &self,
        approved: bool,
        confirm: impl FnMut(&SessionImpact) -> Result<(), DaemonError>,
    ) -> Result<DaemonReadiness, DaemonError> {
        match self.observe().await? {
            ObservedState::NotConfigured => return Err(not_setup_for_command()),
            ObservedState::ConfiguredStopped(_) => {}
            ObservedState::Running(_) => {
                self.stop_with_confirmation(approved, confirm).await?;
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
    use super::*;
    use crate::network::{
        AddressServiceState, NetworkDiagnostic, NetworkObservation, NetworkState,
    };
    use crate::service::ProtocolStatus;
    use zterm_core::{Capabilities, DeviceId};
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
                require_interruption_approval(
                    &update_impact,
                    &mut false,
                    &mut confirmation_required
                )
                .expect_err("update must refuse active Sessions without force")
                .kind(),
                DomainErrorKind::Cancelled
            );
            require_interruption_approval(&update_impact, &mut true, &mut confirmation_required)
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

    #[test]
    fn interruption_approval_reads_once_only_for_actual_impact() {
        let mut approved = false;
        let mut calls = 0;
        let mut confirm = |impact: &SessionImpact| {
            calls += 1;
            assert_eq!(impact.active_session_names, ["main"]);
            Ok(())
        };
        let mut impact = SessionImpact {
            active_session_count: 0,
            active_session_names: Vec::new(),
            stopping: false,
            interruption_required: false,
        };
        require_interruption_approval(&impact, &mut approved, &mut confirm)
            .expect("idle needs no approval");
        assert!(!approved);
        impact.active_session_count = 1;
        impact.active_session_names.push("main".to_owned());
        require_interruption_approval(&impact, &mut approved, &mut confirm)
            .expect("newly admitted work asks");
        require_interruption_approval(&impact, &mut approved, &mut confirm)
            .expect("approval covers this invocation");
        assert_eq!(calls, 1);
        let mut declined = false;
        assert!(
            require_interruption_approval(&impact, &mut declined, &mut confirmation_required)
                .is_err()
        );
        assert!(!declined);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn committed_update_starts_configured_state_and_checks_installed_identity() {
        use zterm_core::release::{ReleaseClassification, ReleaseManifest};
        let installed = ReleaseManifest {
            schema: 1,
            product: "zterm".into(),
            version: "9.1.0".into(),
            tag: "v9.1.0".into(),
            classification: ReleaseClassification::Stable,
            source_commit: "fixture".into(),
            released_at: "fixture".into(),
            wire_major: zterm_core::WIRE_MAJOR,
            state_schema: zterm_core::STATE_SCHEMA_VERSION,
            bootstrap_schema: 1,
            public_key_id: "fixture".into(),
            artifacts: Vec::new(),
        };
        let setup = BootstrapResult {
            device_id: DeviceId::from_array([0x71; 32]),
            endpoint_id: "fixture".into(),
            config: crate::config::validate_setup_profile("update-host", "official-n0", None)
                .expect("valid setup"),
        };
        let stopped = ObservedState::ConfiguredStopped(setup.clone());
        let ready = DaemonReadiness {
            version: installed.version.clone(),
            started_at_unix: 1,
            protocol: ProtocolStatus {
                wire_major: installed.wire_major,
                state_schema: installed.state_schema,
                capabilities: Capabilities::LOCAL_LIFECYCLE,
            },
        };
        let running = ObservedState::Running(DaemonStatus {
            version: ready.version.clone(),
            protocol: ready.protocol,
            phase: "fixture".into(),
            device_id: setup.device_id,
            endpoint_id: setup.endpoint_id.clone(),
            device_name: setup.config.device_name.clone(),
            infrastructure_profile: "official-n0".into(),
            started_at_unix: 1,
            active_session_count: 0,
            active_session_names: Vec::new(),
            network: NetworkObservation::disabled(setup.device_id),
        });
        for observed in [stopped.clone(), running] {
            let mut stages = Vec::new();
            let mut starts = 0;
            assert!(
                finish_update_startup(
                    Ok(observed),
                    &installed,
                    &mut |stage| stages.push(stage),
                    || {
                        starts += 1;
                        std::future::ready(Ok(ready.clone()))
                    }
                )
                .await
                .expect("configured update starts")
            );
            assert_eq!(starts, 1);
            assert_eq!(stages, [UpdateStage::Starting]);
        }
        assert!(
            !finish_update_startup(
                Ok(ObservedState::NotConfigured),
                &installed,
                &mut |_| panic!("no startup phase before setup"),
                || -> std::future::Ready<Result<DaemonReadiness, DaemonError>> {
                    panic!("must not start or create identity")
                }
            )
            .await
            .expect("binary-only update")
        );
        let failure = finish_update_startup(Ok(stopped.clone()), &installed, &mut |_| {}, || {
            std::future::ready(Err(DaemonError::new(
                DomainErrorKind::DaemonStartTimeout,
                "fixture startup failure",
            )))
        })
        .await
        .expect_err("committed startup failure is partial completion");
        assert!(
            failure.to_string().contains("Updated zterm to 9.1.0")
                && failure.to_string().contains("zterm daemon restart")
        );
        for field in 0..3 {
            let mut wrong = ready.clone();
            match field {
                0 => wrong.version = zterm_core::BuildIdentity::current().version.into(),
                1 => wrong.protocol.wire_major += 1,
                _ => wrong.protocol.state_schema += 1,
            }
            assert!(
                finish_update_startup(Ok(stopped.clone()), &installed, &mut |_| {}, || {
                    std::future::ready(Ok(wrong))
                })
                .await
                .is_err()
            );
        }
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
                wire_major: zterm_core::WIRE_MAJOR,
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
        // A just-closed Darwin listener can briefly accept a connection whose
        // peer then observes EOF. Settle that fixture-only state before the
        // reset starts its single production stop deadline.
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match tokio::net::UnixStream::connect(paths.socket()).await {
                    Ok(stream) => {
                        drop(stream);
                        tokio::task::yield_now().await;
                    }
                    Err(error) => {
                        assert_ne!(
                            error.kind(),
                            std::io::ErrorKind::PermissionDenied,
                            "owned stale socket fixture must not reject its owner for permissions"
                        );
                        break;
                    }
                }
            }
        })
        .await
        .expect("closed stale socket listener must stop accepting connections");
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
}
