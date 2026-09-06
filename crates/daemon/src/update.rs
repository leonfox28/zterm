//! One-shot update owner, detached from the terminal it may terminate.
//!
//! Only this module decodes the private parent/child control protocol. Candidate
//! preparation and activation remain in their existing distribution/runtime owners.

use std::fs;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use zterm_core::DomainErrorKind;
use zterm_platform::local_unix::{spawn_isolated_command, take_isolated_channel};
use zterm_platform::user_state::{UserPaths, open_append, validate_directory};

use crate::distribution::{PreparedRelease, ReleaseSelection};
use crate::error::DaemonError;
use crate::operations::{LocalRuntime, UpdateResult, UpdateStage};
use crate::service::SessionImpact;

/// Hidden entry used only by the one-shot update launcher.
pub const INTERNAL_UPDATE_ARGUMENT: &str = "--internal-update";
const MAX_CONTROL_BYTES: usize = 64 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
const READY_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) trait UpdateInteraction {
    fn progress(&mut self, stage: UpdateStage);
    fn confirm(&mut self, impact: &SessionImpact) -> Result<(), DaemonError>;
    fn handoff(&mut self, prepared: &PreparedRelease) -> Result<(), DaemonError>;
    fn committed(&mut self);
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Request {
    Begin {
        version: Option<String>,
        approved: bool,
    },
    Approve,
    Cancel,
    Continue,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Event {
    Ready,
    Progress(UpdateStage),
    Confirm(SessionImpact),
    Handoff,
    Accepted,
    Complete(Result<UpdateResult, Failure>),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Failure {
    code: String,
    detail: String,
}

impl From<DaemonError> for Failure {
    fn from(error: DaemonError) -> Self {
        Self {
            code: error.kind().code().into(),
            detail: error.detail().into(),
        }
    }
}

impl Failure {
    fn into_error(self) -> DaemonError {
        DaemonError::new(
            DomainErrorKind::from_code(&self.code).unwrap_or(DomainErrorKind::MalformedFrame),
            self.detail,
        )
    }
}

fn channel_error() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::OperationOutcomeUnknown,
        "Update control connection closed or failed. If the update was accepted, it may still be running; check zterm --version, zterm daemon status and zterm logs before retrying.",
    )
}

fn cancelled() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::Cancelled,
        "Update cancelled before interruption approval or execution handoff.",
    )
}

struct Channel {
    stream: UnixStream,
    usable: bool,
}

impl Channel {
    fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            usable: true,
        }
    }

    fn send(&mut self, value: &impl Serialize) -> Result<(), DaemonError> {
        let result = self.write(value);
        if result.is_err() {
            self.usable = false;
            // A partial frame must never be followed by another frame.
            let _ = self.stream.shutdown(Shutdown::Both);
        }
        result
    }

    fn write(&mut self, value: &impl Serialize) -> Result<(), DaemonError> {
        if !self.usable {
            return Err(channel_error());
        }
        let body = serde_json::to_vec(value).map_err(|_| channel_error())?;
        if body.len() > MAX_CONTROL_BYTES {
            return Err(channel_error());
        }
        let mut bytes = Vec::with_capacity(4 + body.len());
        bytes.extend_from_slice(
            &u32::try_from(body.len())
                .map_err(|_| channel_error())?
                .to_be_bytes(),
        );
        bytes.extend_from_slice(&body);
        let deadline = Instant::now() + WRITE_TIMEOUT;
        let mut remaining = bytes.as_slice();
        while !remaining.is_empty() {
            let timeout = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(channel_error)?;
            self.stream
                .set_write_timeout(Some(timeout))
                .map_err(|_| channel_error())?;
            match self.stream.write(remaining) {
                Ok(0) => return Err(channel_error()),
                Ok(count) => remaining = &remaining[count..],
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Err(channel_error()),
            }
        }
        Ok(())
    }

    fn receive<T: DeserializeOwned>(&mut self) -> Result<T, DaemonError> {
        if !self.usable {
            return Err(channel_error());
        }
        let mut length = [0; 4];
        self.stream
            .read_exact(&mut length)
            .map_err(|_| channel_error())?;
        let length = u32::from_be_bytes(length) as usize;
        if length == 0 || length > MAX_CONTROL_BYTES {
            return Err(channel_error());
        }
        let mut bytes = vec![0; length];
        self.stream
            .read_exact(&mut bytes)
            .map_err(|_| channel_error())?;
        serde_json::from_slice(&bytes).map_err(|_| channel_error())
    }
}

// A child that never became ready cannot mutate and may be terminated.
// Once ready, let channel EOF/cancellation unwind its owned staging normally;
// after Continue, the child must also finish any accepted mutation independently.
struct UpdaterChild {
    child: Option<Child>,
    continued: bool,
    ready: bool,
}

impl Drop for UpdaterChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if !self.ready {
                let _ = child.kill();
            }
            let _ = std::thread::Builder::new()
                .name("zterm-update-reap".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }
}

pub(crate) fn run_frontend(
    executable: &Path,
    paths: &UserPaths,
    version: Option<&str>,
    approved: bool,
    mut confirm: impl FnMut(&SessionImpact) -> Result<(), DaemonError>,
    mut progress: impl FnMut(UpdateStage),
) -> Result<UpdateResult, DaemonError> {
    let (child, stream) =
        spawn_isolated_command(executable, paths.home(), INTERNAL_UPDATE_ARGUMENT).map_err(
            |_| {
                DaemonError::new(
                    DomainErrorKind::UpdateRejected,
                    "Unable to start the independent updater; the daemon has not been stopped.",
                )
            },
        )?;
    let mut child = UpdaterChild {
        child: Some(child),
        continued: false,
        ready: false,
    };
    let mut channel = Channel::new(stream);
    let result = (|| {
        channel
            .stream
            .set_read_timeout(Some(READY_TIMEOUT))
            .map_err(|_| channel_error())?;
        if !matches!(channel.receive()?, Event::Ready) {
            return Err(channel_error());
        }
        child.ready = true;
        channel
            .stream
            .set_read_timeout(None)
            .map_err(|_| channel_error())?;
        channel.send(&Request::Begin {
            version: version.map(str::to_owned),
            approved,
        })?;
        loop {
            match channel.receive()? {
                Event::Progress(stage) => progress(stage),
                Event::Confirm(impact) => {
                    if let Err(error) = confirm(&impact) {
                        let _ = channel.send(&Request::Cancel);
                        return Err(error);
                    }
                    channel.send(&Request::Approve)?;
                }
                Event::Handoff if !child.continued => {
                    progress(UpdateStage::Continuing);
                    child.continued = true;
                    channel.send(&Request::Continue)?;
                }
                Event::Accepted if child.continued => {}
                Event::Complete(result) => return result.map_err(Failure::into_error),
                _ => return Err(channel_error()),
            }
        }
    })();
    result.map_err(|error: DaemonError| {
        if !child.continued && error.kind() == DomainErrorKind::OperationOutcomeUnknown {
            DaemonError::new(DomainErrorKind::Cancelled, "Update control failed before execution handoff; this updater did not stop the daemon or change the installed binary.")
        } else {
            error
        }
    })
}

/// Runs the hidden one-shot updater. It never obtains terminal input directly.
pub fn run_internal_update() -> Result<(), DaemonError> {
    let paths = crate::lifecycle::production_user_paths()?;
    let stream = take_isolated_channel(paths.uid()).map_err(|_| channel_error())?;
    let runtime = LocalRuntime::current()?;
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| channel_error())?;
    let uid = paths.uid();
    tokio.block_on(run_worker(&runtime, &paths, stream, |version| async move {
        let executable = std::env::current_exe().map_err(|_| channel_error())?;
        crate::distribution::validate_managed_executable(&executable, uid)?;
        crate::distribution::prepare_update(ReleaseSelection::parse(version.as_deref())?).await
    }))
}

async fn run_worker<F, Fut>(
    runtime: &LocalRuntime,
    paths: &UserPaths,
    stream: UnixStream,
    prepare: F,
) -> Result<(), DaemonError>
where
    F: FnOnce(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<PreparedRelease, DaemonError>>,
{
    let mut worker = Worker {
        channel: Channel::new(stream),
        paths,
        version: None,
        log_enabled: false,
        accepted: false,
        committed: false,
    };
    worker.channel.send(&Event::Ready)?;
    let Request::Begin { version, approved } = worker.channel.receive()? else {
        return Err(cancelled());
    };
    let result = async {
        worker.progress(UpdateStage::Preparing);
        let prepared = prepare(version).await?;
        worker.progress(UpdateStage::Verified {
            version: prepared.version().to_owned(),
        });
        runtime
            .apply_prepared_update(prepared, approved, &mut worker)
            .await
    }
    .await;
    let record = match &result {
        Ok(result) => format!(
            "outcome=success installed={} daemon_started={}",
            result.installed_version, result.daemon_started
        ),
        Err(error) => format!(
            "outcome={} category={} guidance=check-installed-version-and-daemon-status;use-SSH-if-disconnected;restart-daemon-if-activation-committed",
            if worker.committed {
                "partial_completion"
            } else {
                "failed"
            },
            error.kind().code()
        ),
    };
    let _ = worker.log(&record);
    let _ = worker
        .channel
        .send(&Event::Complete(result.clone().map_err(Into::into)));
    result.map(|_| ())
}

struct Worker<'a> {
    channel: Channel,
    paths: &'a UserPaths,
    version: Option<String>,
    log_enabled: bool,
    accepted: bool,
    committed: bool,
}

impl Worker<'_> {
    fn log(&self, record: &str) -> Result<(), DaemonError> {
        if !self.log_enabled {
            return Ok(());
        }
        let result = (|| {
            validate_directory(self.paths.state_root(), self.paths.uid())?;
            validate_directory(self.paths.logs(), self.paths.uid())?;
            open_append(self.paths.daemon_log(), self.paths.uid())
        })()
        .map_err(|_| {
            DaemonError::new(
                DomainErrorKind::PathUnsafe,
                "Unable to open the update outcome log.",
            )
        })?;
        let mut file = result;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        writeln!(
            file,
            "{timestamp} INFO update pid={} target={} accepted={} {record}",
            std::process::id(),
            self.version.as_deref().unwrap_or("unknown"),
            self.accepted
        )
        .and_then(|()| file.sync_data())
        .map_err(|_| {
            DaemonError::new(
                DomainErrorKind::PathUnsafe,
                "Unable to record the update outcome.",
            )
        })
    }
}

impl UpdateInteraction for Worker<'_> {
    fn progress(&mut self, stage: UpdateStage) {
        let code = match &stage {
            UpdateStage::Preparing => "preparing",
            UpdateStage::Verified { .. } => "verified",
            UpdateStage::Continuing => "continuing",
            UpdateStage::Stopping => "stopping",
            UpdateStage::Activating => "activating",
            UpdateStage::Starting => "starting",
        };
        let _ = self.log(&format!("stage={code}"));
        let _ = self.channel.send(&Event::Progress(stage));
    }

    fn confirm(&mut self, impact: &SessionImpact) -> Result<(), DaemonError> {
        self.channel.send(&Event::Confirm(impact.clone()))?;
        match self.channel.receive()? {
            Request::Approve => Ok(()),
            _ => Err(cancelled()),
        }
    }

    fn committed(&mut self) {
        self.committed = true;
        let _ = self.log("stage=committed");
    }

    fn handoff(&mut self, prepared: &PreparedRelease) -> Result<(), DaemonError> {
        self.version = Some(prepared.version().to_owned());
        self.log_enabled = match fs::symlink_metadata(self.paths.state_root()) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => {
                return Err(DaemonError::new(
                    DomainErrorKind::PathUnsafe,
                    "Unable to inspect update state.",
                ));
            }
        };
        self.log("stage=awaiting-handoff")?;
        self.channel.send(&Event::Handoff)?;
        if !matches!(self.channel.receive()?, Request::Continue) {
            return Err(cancelled());
        }
        self.accepted = true;
        self.log("stage=accepted")?;
        // The received Continue transfers ownership even if its ACK is lost.
        let _ = self.channel.send(&Event::Accepted);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
