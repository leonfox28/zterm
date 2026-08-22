//! Daemon-aware command backend shared by the thin CLI.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Instant;

use zterm_core::DomainErrorKind;
use zterm_platform::user_state::UserPaths;

use crate::bootstrap::{BootstrapResult, bootstrap_with_lock_held};
use crate::config::ValidatedConfig;
use crate::error::DaemonError;
use crate::lifecycle::{DaemonLauncher, acquire_lifecycle_lock, probe_readiness};
use crate::local_ipc::LocalClient;
use crate::service::{DaemonReadiness, DaemonStatus, SessionImpact};

const MAX_LOG_LINES: usize = 1_000;
const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Side-effect-free observation of local setup and daemon state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservedState {
    /// Setup is complete and the daemon answered status.
    Running(DaemonStatus),
    /// Setup is complete but no daemon is listening.
    ConfiguredStopped(BootstrapResult),
    /// No complete setup exists.
    NotConfigured,
}

/// One diagnostic check from the non-spawning doctor command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoctorCheck {
    /// Stable check name.
    pub name: &'static str,
    /// Whether the check passed.
    pub ok: bool,
    /// Bounded human-readable detail.
    pub detail: String,
}

/// Non-spawning local diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoctorReport {
    /// True when every required local-state check passed.
    pub healthy: bool,
    /// Ordered diagnostic checks.
    pub checks: Vec<DoctorCheck>,
}

/// Daemon-owned paths and launcher used by one CLI invocation.
#[derive(Clone, Debug)]
pub struct LocalRuntime {
    paths: UserPaths,
    launcher: DaemonLauncher,
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

    /// Returns the persistent state root for diagnostics and rendering.
    #[must_use]
    pub fn state_root(&self) -> &std::path::Path {
        self.paths.state_root()
    }

    /// Returns the managed daemon log path.
    #[must_use]
    pub fn daemon_log_path(&self) -> &std::path::Path {
        self.paths.daemon_log()
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

    /// Explicitly ensures one daemon for restart/setup flows.
    pub async fn ensure(&self) -> Result<DaemonReadiness, DaemonError> {
        self.launcher.ensure(&self.paths).await
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
        if self.stop(force).await?.is_some() {
            wait_until_stopped(&self.paths).await?;
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
