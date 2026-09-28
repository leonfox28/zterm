//! Desktop composition and an allowlisted adapter for existing owner events.

use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tracing::field::{Field, Visit};
use tracing_subscriber::{Layer, layer::Context, prelude::*};
use zterm_core::{DomainErrorKind, SessionId};
use zterm_diagnostics::{Event, Kind, Level, Recorder, Stage};
use zterm_platform::{diagnostics::Store, user_state::UserPaths};

// The lifecycle registration and recorder must share the same storage adapter
// so a transient marker-write failure can be repaired by subsequent appends.
static STORE: OnceLock<(UserPaths, Store)> = OnceLock::new();

/// Creates an injected recorder only when committed setup exists.
/// The worker alone performs append/rotation; this does not initialize identity.
pub(crate) fn recorder(paths: &UserPaths) -> Recorder {
    if let Some(recorder) = zterm_diagnostics::global_recorder() {
        return recorder;
    }
    if !configured(paths) {
        return Recorder::default();
    }
    Recorder::new(Arc::new(Store::new(paths.clone()))).unwrap_or_default()
}

/// Installs one process recorder and the safe application tracing adapter.
/// Returns an explicit bounded-flush guard; callers retain it until exit.
pub fn install(paths: &UserPaths) -> Guard {
    let recorder = zterm_diagnostics::global_recorder().unwrap_or_else(|| {
        if !configured(paths) {
            return Recorder::default();
        }
        let store = Store::new(paths.clone());
        let recorder = Recorder::new(Arc::new(store.clone())).unwrap_or_default();
        let _ = STORE.set((paths.clone(), store));
        recorder
    });
    if zterm_diagnostics::install(recorder.clone()) {
        let _ = tracing_subscriber::registry()
            .with(ApplicationEvents(recorder.clone()))
            .try_init();
    }
    Guard(recorder)
}

pub(crate) fn register_daemon(
    paths: &UserPaths,
    lock: &zterm_platform::local_unix::DaemonLock,
) -> std::io::Result<()> {
    if let Some((installed, store)) = STORE.get()
        && installed.state_root() == paths.state_root()
        && installed.uid() == paths.uid()
    {
        store.register_daemon(lock)
    } else {
        Store::new(paths.clone()).register_daemon(lock)
    }
}
/// Flush guard for process owners, never attached to a terminal actor.
pub struct Guard(Recorder);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.flush(Duration::from_secs(2));
    }
}

pub(crate) struct ApplicationEvents(pub(crate) Recorder);
impl<S: tracing::Subscriber> Layer<S> for ApplicationEvents {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if !event.metadata().target().starts_with("zterm_daemon::") {
            return;
        }
        let mut fields = OwnerFields::default();
        event.record(&mut fields);
        let kind = match (fields.component.as_str(), fields.operation.as_str()) {
            ("daemon", "ready") => Kind::DaemonReady,
            ("daemon", "stopping") => Kind::DaemonStopping,
            ("daemon", "listener_failed") => Kind::ListenerFailed,
            ("daemon", "listener_recovered") => Kind::ListenerRecovered,
            ("daemon", "cleanup_failed") => Kind::CleanupFailed,
            ("session", "created") => Kind::SessionCreated,
            ("session", "renamed") => Kind::SessionRenamed,
            ("session", "ended") => Kind::SessionEnded,
            ("session", "cleanup_failed") => Kind::SessionCleanupFailed,
            ("session", "controller_attached") => Kind::ControllerAttached,
            ("session", "controller_detached") => Kind::ControllerDetached,
            ("session", "controller_taken_over") => Kind::ControllerTakenOver,
            ("connection", "primary_established") => Kind::PrimaryEstablished,
            ("connection", "primary_closed") => Kind::PrimaryClosed,
            ("network", "state_changed") => Kind::NetworkChanged,
            ("pairing", "offer_created") => Kind::PairOfferCreated,
            ("pairing", "offer_failed") => Kind::PairOfferFailed,
            ("pairing", "accept_committed") => Kind::PairAcceptCommitted,
            ("pairing", "accept_failed") => Kind::PairAcceptFailed,
            ("pairing", "inbound_authorized") => Kind::InboundAuthorized,
            ("authorization", "revoke_failed") => Kind::RequestFailed,
            _ => return,
        };
        let level = match *event.metadata().level() {
            tracing::Level::ERROR => Level::Error,
            tracing::Level::WARN => Level::Warn,
            tracing::Level::INFO => Level::Info,
            _ => Level::Debug,
        };
        let mut record = Event::new(kind)
            .level(level)
            .exit_code(fields.exit_code)
            .signal(fields.signal);
        if let Some(session) = fields.session {
            record = record.session(session);
        }
        if let Some(attachment) = fields.attachment {
            record = record.attachment(attachment);
        }
        if let Some(error) = DomainErrorKind::from_code(&fields.reason) {
            record = record.error(error);
        }
        if let Ok(failure) =
            serde_json::from_value(serde_json::Value::String(fields.reason.clone()))
        {
            record = record.network_failure(failure);
        }
        if let Ok(reason) = serde_json::from_value(serde_json::Value::String(fields.reason)) {
            record = record.reason(reason);
        }
        let stage = match fields.state.as_str() {
            "degraded" => Some(Stage::Degraded),
            "ready" | "online" => Some(Stage::Ready),
            "starting" => Some(Stage::Starting),
            "initializing" => Some(Stage::Initializing),
            "bound" => Some(Stage::Bound),
            "disabled" => Some(Stage::Disabled),
            "stopping" => Some(Stage::Stopping),
            "stopped" => Some(Stage::Closed),
            _ => None,
        };
        if let Some(stage) = stage {
            record = record.stage(stage);
        }
        if kind == Kind::NetworkChanged {
            record = record.address_services(fields.publish, fields.lookup);
        }
        if let Some(connection) = fields.connection {
            record = record.connection(connection);
        }
        if let Some(count) = fields.count {
            record = record.count(count);
        }
        self.0.record(record);
    }
}
#[derive(Default)]
struct OwnerFields {
    component: String,
    operation: String,
    reason: String,
    state: String,
    publish: Option<zterm_diagnostics::AddressService>,
    lookup: Option<zterm_diagnostics::AddressService>,
    session: Option<SessionId>,
    attachment: Option<zterm_core::AttachmentId>,
    exit_code: Option<i32>,
    signal: Option<i32>,
    connection: Option<u64>,
    count: Option<u64>,
}
impl Visit for OwnerFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "component" if value.len() < 32 => self.component = value.into(),
            "operation" if value.len() < 64 => self.operation = value.into(),
            "reason" | "error_kind" if value.len() < 64 => self.reason = value.into(),
            "state" if value.len() < 32 => self.state = value.into(),
            "publish" | "lookup" => {
                let state = match value {
                    "disabled" => Some(zterm_diagnostics::AddressService::Disabled),
                    "configured" => Some(zterm_diagnostics::AddressService::Configured),
                    "degraded" => Some(zterm_diagnostics::AddressService::Degraded),
                    _ => None,
                };
                if field.name() == "publish" {
                    self.publish = state;
                } else {
                    self.lookup = state;
                }
            }
            "session_id" => self.session = value.parse().ok(),
            "attachment_id" => {
                self.attachment = value
                    .parse::<SessionId>()
                    .ok()
                    .map(|id| zterm_core::AttachmentId::from_array(id.to_bytes()))
            }
            _ => {} // In particular: message, name, targets, addresses and errors.
        }
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        match field.name() {
            "connection" => self.connection = Some(value),
            "generation" | "ttl_seconds" => self.count = Some(value),
            _ => {}
        }
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        match field.name() {
            "exit_code" => self.exit_code = i32::try_from(value).ok(),
            "signal" => self.signal = i32::try_from(value).ok(),
            _ => {}
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        // Display values are visited through Debug by tracing. Only canonical
        // public IDs have a text conversion path; arbitrary Debug is discarded.
        if matches!(field.name(), "session_id" | "attachment_id") {
            self.record_str(field, &format!("{value:?}"));
        }
    }
}

/// The final owned config pathname is setup's commit marker. Logging must still
/// work when identity/database contents are the reason startup is failing.
pub(crate) fn configured(paths: &UserPaths) -> bool {
    zterm_platform::user_state::validate_directory(paths.state_root(), paths.uid()).is_ok()
        && zterm_platform::user_state::validate_regular_file(paths.config(), paths.uid()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_worker_repairs_its_daemon_registration() {
        const CHILD: &str = "ZTERM_DIAGNOSTICS_COMPOSITION_FIXTURE";
        if std::env::var_os(CHILD).is_none() {
            let output =
                std::process::Command::new(std::env::current_exe().expect("test executable"))
                    .args([
                        "--exact",
                        "diagnostics::tests::installed_worker_repairs_its_daemon_registration",
                    ])
                    .env(CHILD, "1")
                    .output()
                    .expect("isolated process composition");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            return;
        }
        let temp = tempfile::tempdir().expect("private root");
        let paths = UserPaths::for_test(
            nix::unistd::geteuid().as_raw(),
            temp.path().into(),
            temp.path().join("state"),
            temp.path().join("run"),
        );
        paths.prepare_state_directories().expect("paths");
        zterm_platform::user_state::atomic_write(paths.config(), paths.uid(), |_| Ok(()))
            .expect("owned configuration commit marker");
        let _guard = install(&paths);
        let lock = zterm_platform::local_unix::DaemonLock::try_acquire(&paths)
            .expect("daemon lock")
            .expect("exclusive");
        let (_listener, _ownership) =
            zterm_platform::local_unix::bind_owned_daemon_socket(&paths, &lock).expect("socket");
        register_daemon(&paths, &lock).expect("registration");
        std::fs::remove_file(paths.logs().join("diagnostics.json")).expect("lost control");
        zterm_diagnostics::record(Event::new(Kind::DaemonReady));
        assert!(zterm_diagnostics::flush(Duration::from_secs(2)));
        assert!(
            !Store::new(paths)
                .retention_pending()
                .expect("worker repaired marker")
        );
    }
}
