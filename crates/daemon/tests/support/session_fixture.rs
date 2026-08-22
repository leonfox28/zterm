#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use zterm_core::terminal::{TerminalModel, TerminalSize, TerminalSnapshot};
use zterm_core::{
    AttachmentId, AttachmentPrincipal, DeviceId, DomainErrorKind, OperationId, OperationLease,
    ResourceLimits, Revision, SessionName,
};
use zterm_daemon::error::DaemonError;
use zterm_daemon::session::{PreparedAttachment, SessionAttachment, SessionService};
use zterm_platform::pty::{ExplicitPtyCommand, PtyHost, PtySize};

pub const DEADLINE: Duration = Duration::from_secs(10);

pub struct Fixture {
    pub temp: TempDir,
    pub service: SessionService,
    pub principal: AttachmentPrincipal,
    operation_lease: OperationLease,
}

impl Fixture {
    pub fn new(limits: ResourceLimits) -> Result<Self, String> {
        let temp = tempfile::tempdir().map_err(display)?;
        let service = service_for_path(temp.path().to_path_buf(), limits)?;
        let principal = service.local_principal(AttachmentId::from_array([9; 16]));
        let operation_lease = service.issue_operation_lease(principal).map_err(display)?;
        Ok(Self {
            temp,
            service,
            principal,
            operation_lease,
        })
    }

    pub fn op(&self, sequence: u64) -> OperationId {
        OperationId {
            lease: self.operation_lease,
            sequence,
        }
    }

    pub fn create(
        &self,
        sequence: u64,
        name: &str,
    ) -> Result<zterm_daemon::session::SessionSummary, DaemonError> {
        self.service.create(
            self.principal,
            self.op(sequence),
            SessionName::new(name).expect("fixture name"),
            None,
            None,
        )
    }
}

pub fn service_for_path(
    default_cwd: PathBuf,
    limits: ResourceLimits,
) -> Result<SessionService, String> {
    let default_cwd = Arc::new(default_cwd);
    let shell = shell_path()?;
    Ok(SessionService::with_spawner(
        DeviceId::from_array([7; 32]),
        limits,
        move |size, requested| {
            let working_directory = requested
                .map(Path::to_path_buf)
                .unwrap_or_else(|| default_cwd.as_ref().clone());
            let command = ExplicitPtyCommand::new(&shell, &working_directory).arg("-i");
            let session = PtyHost::new()
                .spawn(command, PtySize::new(size.rows, size.columns))
                .map_err(|error| {
                    DaemonError::new(DomainErrorKind::InvalidWorkingDirectory, error.to_string())
                })?;
            Ok((session, working_directory))
        },
    ))
}

pub fn activate(prepared: &PreparedAttachment) -> Result<(), String> {
    let replacement = prepared
        .attachment
        .snapshot_applied(prepared.snapshot.revision)
        .map_err(display)?;
    if replacement.is_some() {
        return Err("exact initial snapshot unexpectedly required replacement".into());
    }
    Ok(())
}

pub fn latest_snapshot(attachment: &SessionAttachment) -> Result<TerminalSnapshot, String> {
    let snapshot = attachment.sync_latest(Revision::ZERO).map_err(display)?;
    let replacement = attachment
        .snapshot_applied(snapshot.revision)
        .map_err(display)?;
    if replacement.is_some() {
        return Err("exact resync acknowledgement unexpectedly required replacement".into());
    }
    Ok(snapshot)
}

pub fn wait_for_text(attachment: &SessionAttachment, needle: &str) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let snapshot = latest_snapshot(attachment)?;
        let text = snapshot_text(&snapshot)?;
        if text.contains(needle) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "terminal never contained {needle:?}; tail={:?}",
                text
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn wait_for_session_count(service: &SessionService, expected: usize) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let sessions = service.list().map_err(display)?;
        if sessions.len() == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "session count remained {}, expected {expected}",
                sessions.len()
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn snapshot_text(snapshot: &TerminalSnapshot) -> Result<String, String> {
    let mut client = TerminalModel::new(snapshot.size, 2_000).map_err(display)?;
    client
        .ingest(&snapshot.recent_history_ansi)
        .map_err(display)?;
    client.ingest(&snapshot.screen_ansi).map_err(display)?;
    let state = client.state();
    let columns = usize::from(state.size.columns);
    let mut output = String::new();
    for row in state.cells.chunks(columns) {
        for cell in row {
            output.push_str(&cell.contents);
        }
        output.push('\n');
    }
    Ok(output)
}

pub fn default_size() -> TerminalSize {
    TerminalSize::new(40, 120)
}

fn shell_path() -> Result<PathBuf, String> {
    ["/bin/sh", "/usr/bin/sh"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .ok_or_else(|| "no absolute POSIX shell fixture is available".into())
}

pub fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}
