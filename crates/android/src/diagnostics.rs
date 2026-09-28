//! Typed, application-owned diagnostics bridge; callbacks run on the Rust worker.

use std::sync::Arc;
use std::time::Duration;
use zterm_diagnostics::{
    Control, Event, FrontendFailure, Kind, Level, Outcome, Recorder, Sink, Stage,
};

/// Finite interval persisted by the Android storage owner.
#[derive(Clone, uniffi::Record)]
pub struct DiagnosticControl {
    /// Version-one marker.
    pub schema: u8,
    /// Explicit enable time, UTC Unix milliseconds.
    pub started_ms: i64,
    /// Expiry, UTC Unix milliseconds (zero means off).
    pub deadline_ms: i64,
}
impl From<Control> for DiagnosticControl {
    fn from(c: Control) -> Self {
        Self {
            schema: c.schema,
            started_ms: c.started_ms,
            deadline_ms: c.deadline_ms,
        }
    }
}
impl From<DiagnosticControl> for Control {
    fn from(c: DiagnosticControl) -> Self {
        Self {
            schema: c.schema,
            started_ms: c.started_ms,
            deadline_ms: c.deadline_ms,
        }
    }
}
/// Only the Application's private storage implements this callback. Never reenter diagnostics.
#[uniffi::export(callback_interface)]
pub trait DiagnosticSink: Send + Sync {
    /// Append a complete bounded record off Main; false reports loss.
    fn append(&self, detail: bool, record: String) -> bool;
    /// Read the small interval marker off Main, even when the recorder is idle.
    fn control(&self) -> DiagnosticControl;
    /// Durability barrier, invoked only on the diagnostics worker.
    fn flush(&self) -> bool;
}
struct AndroidSink(Box<dyn DiagnosticSink>);
impl Sink for AndroidSink {
    fn append(&self, detail: bool, bytes: &[u8]) -> std::io::Result<()> {
        if self
            .0
            .append(detail, String::from_utf8_lossy(bytes).into_owned())
        {
            Ok(())
        } else {
            Err(std::io::ErrorKind::Other.into())
        }
    }
    fn control(&self) -> std::io::Result<Control> {
        Ok(self.0.control().into())
    }
    fn flush(&self) -> std::io::Result<()> {
        if self.0.flush() {
            Ok(())
        } else {
            Err(std::io::ErrorKind::Other.into())
        }
    }
}
/// Closed Kotlin event vocabulary; no string messages, exception trees or content.
#[derive(Clone, Copy, uniffi::Enum)]
pub enum AppDiagnostic {
    /// Cold application start.
    Started,
    /// Application became visible.
    Foreground,
    /// Application became hidden; networking remains alive.
    Background,
    /// Network configuration changed (no addresses).
    NetworkChanged,
    /// Identity/network initialization completed.
    Initialized,
    /// Initialization failed.
    InitializationFailed,
    /// User operation failed.
    OperationFailed,
    /// Local storage failed.
    StorageFailed,
    /// Notification posting failed.
    NotificationFailed,
    /// Update preparation started.
    UpdateStarted,
    /// Update validation completed.
    UpdateVerified,
    /// Installer handoff occurred, not installation success.
    UpdateHandedOff,
    /// Update preparation/download failed.
    UpdateFailed,
    /// Input admission opened.
    InputReady,
    /// Input admission closed.
    InputBlocked,
    /// Viewport dimensions changed.
    Viewport,
    /// Explicit export finished.
    ExportCompleted,
    /// Explicit export failed.
    ExportFailed,
}
/// Application-owned recorder handle. Activity recreation must not replace it.
#[derive(uniffi::Object)]
pub struct NativeDiagnostics {
    recorder: Recorder,
}
#[uniffi::export]
impl NativeDiagnostics {
    /// Emits a typed observation. Category is accepted only if it is a known domain code.
    pub fn record_app(
        &self,
        event: AppDiagnostic,
        category: Option<String>,
        columns: u16,
        rows: u16,
    ) {
        let event = match event {
            AppDiagnostic::Started => Event::new(Kind::AppStarted),
            AppDiagnostic::Foreground => Event::new(Kind::AppLifecycle).stage(Stage::Foreground),
            AppDiagnostic::Background => Event::new(Kind::AppLifecycle).stage(Stage::Background),
            AppDiagnostic::NetworkChanged => Event::new(Kind::NetworkChanged),
            AppDiagnostic::Initialized => {
                Event::new(Kind::NativeInitialized).outcome(Outcome::Success)
            }
            AppDiagnostic::InitializationFailed => Event::new(Kind::NativeInitialized)
                .level(Level::Error)
                .outcome(Outcome::Failed),
            AppDiagnostic::OperationFailed => Event::new(Kind::AppOperationFailed)
                .level(Level::Warn)
                .outcome(Outcome::Failed),
            AppDiagnostic::StorageFailed => Event::new(Kind::StorageFailed)
                .level(Level::Warn)
                .frontend_error(FrontendFailure::Storage),
            AppDiagnostic::NotificationFailed => Event::new(Kind::NotificationFailed)
                .level(Level::Warn)
                .frontend_error(FrontendFailure::Notification),
            AppDiagnostic::UpdateStarted => Event::new(Kind::UpdateStage).stage(Stage::Preparing),
            AppDiagnostic::UpdateVerified => Event::new(Kind::UpdateStage).stage(Stage::Verified),
            AppDiagnostic::UpdateHandedOff => Event::new(Kind::UpdateStage).stage(Stage::Accepted),
            AppDiagnostic::UpdateFailed => Event::new(Kind::UpdateCompleted)
                .level(Level::Warn)
                .outcome(Outcome::Failed),
            AppDiagnostic::InputReady => Event::new(Kind::InputFenceChanged)
                .level(Level::Debug)
                .stage(Stage::InputReady),
            AppDiagnostic::InputBlocked => Event::new(Kind::InputFenceChanged)
                .level(Level::Debug)
                .stage(Stage::InputBlocked),
            AppDiagnostic::Viewport => Event::new(Kind::ViewportChanged)
                .level(Level::Debug)
                .viewport(columns, rows),
            AppDiagnostic::ExportCompleted => Event::new(Kind::ExportCompleted),
            AppDiagnostic::ExportFailed => Event::new(Kind::ExportFailed)
                .level(Level::Warn)
                .frontend_error(FrontendFailure::Export),
        };
        let code = category.as_deref().and_then(|code| {
            use zterm_core::DomainErrorKind as D;
            D::from_code(code).or(match code {
                "update_network" => Some(D::ReleaseUnavailable),
                "update_signature" => Some(D::ReleaseSignatureInvalid),
                "update_download_failed" | "update_invalid" => Some(D::ReleaseArtifactInvalid),
                "update_storage" | "storage_unavailable" => Some(D::StoreUnavailable),
                "input_not_ready" | "selection_changed" => Some(D::NotSynchronized),
                "resource_limit" => Some(D::ResourceExhausted),
                "history_unavailable" => Some(D::TransportUnavailable),
                _ => None,
            })
        });
        let event = if let Some(code) = code {
            event.error(code)
        } else {
            event
        };
        self.recorder.record(event);
    }
    /// Cheap current detail gate; no filesystem access.
    pub fn detail_enabled(&self) -> bool {
        self.recorder.detail_enabled()
    }
    /// Pending loss count for the export header.
    pub fn pending_lost(&self) -> u64 {
        self.recorder.lost()
    }
    /// Bounded flush for export/tests, called from Dispatchers.IO, never Main.
    pub fn flush(&self) -> bool {
        self.recorder.flush(Duration::from_secs(2))
    }
}
/// Installs diagnostics before constructing identity/network owners.
#[uniffi::export]
pub fn install_diagnostics(sink: Box<dyn DiagnosticSink>) -> Arc<NativeDiagnostics> {
    let recorder = Recorder::new(Arc::new(AndroidSink(sink))).unwrap_or_default();
    if zterm_diagnostics::install(recorder.clone()) {
        std::panic::set_hook(Box::new(|_| {
            zterm_diagnostics::record(Event::new(Kind::ProcessPanicked).level(Level::Error));
            let _ = zterm_diagnostics::flush(Duration::from_millis(250));
        }));
    }
    Arc::new(NativeDiagnostics { recorder })
}
/// Builds a marker for an explicit user action; Kotlin persists it atomically.
#[uniffi::export]
pub fn diagnostic_control(enabled: bool) -> DiagnosticControl {
    if enabled {
        Control::enabled(zterm_diagnostics::now_ms())
    } else {
        Control::disabled()
    }
    .into()
}
/// Validates a persisted interval for Settings without altering identity/configuration.
#[uniffi::export]
pub fn diagnostic_remaining_ms(control: DiagnosticControl) -> u64 {
    Control::from(control).remaining_ms(zterm_diagnostics::now_ms())
}
/// Validates a bounded input line and returns canonical safe JSONL for export.
#[uniffi::export]
pub fn diagnostic_export_record(line: Vec<u8>) -> Option<String> {
    zterm_diagnostics::Record::decode(&line)
        .and_then(|record| record.encode())
        .and_then(|bytes| String::from_utf8(bytes).ok())
}
/// Shared safe export header; no device identity, address book or configuration.
#[uniffi::export]
pub fn diagnostic_export_header(
    include_debug: bool,
    omitted: u64,
    truncated_bytes: u64,
    pending_lost: u64,
) -> String {
    zterm_diagnostics::ExportHeader::new(include_debug, omitted, truncated_bytes, pending_lost)
        .encode()
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}
