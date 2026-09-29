//! Closed event taxonomy and the versioned on-disk contract.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use zterm_core::{AttachmentId, DomainErrorKind, SessionId};

/// Maximum encoded record, including its newline.
pub const MAX_RECORD_BYTES: usize = 4096;
/// Maximum records in either producer queue.
pub const QUEUE_RECORDS: usize = 256;
/// Maximum serialized bytes in either producer queue.
pub const QUEUE_BYTES: usize = 512 * 1024;
/// Explicit diagnostic interval, never renewed implicitly.
pub const DETAIL_SECONDS: i64 = 15 * 60;

/// Severity in increasing order.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Optional diagnostic detail.
    Debug,
    /// Ordinary transition or outcome.
    Info,
    /// Actionable recoverable failure.
    Warn,
    /// Failure preventing continuation.
    Error,
}

macro_rules! kinds {
    ($($variant:ident => ($component:literal, $message:literal)),+ $(,)?) => {
        /// Recognized application events; dependency messages are never admitted.
        #[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
        #[serde(rename_all = "snake_case")]
        pub enum Kind { $(#[doc = $message] $variant),+ }
        impl Kind {
            /// Stable owner category.
            pub const fn component(self) -> &'static str {
                match self { $(Self::$variant => $component),+ }
            }
            /// Fixed English description; never peer-authored text.
            pub const fn message(self) -> &'static str {
                match self { $(Self::$variant => $message),+ }
            }
        }
    };
}
kinds! {
    ProcessStarted => ("process", "Process started"),
    ProcessStopped => ("process", "Process stopped"),
    ProcessFailed => ("process", "Process failed"),
    ProcessPanicked => ("process", "Process panicked"),
    DaemonReady => ("daemon", "Local daemon ready"),
    DaemonStopping => ("daemon", "Local daemon stopping"),
    ListenerFailed => ("daemon", "Listener failed"),
    ListenerRecovered => ("daemon", "Listener recovered"),
    CleanupFailed => ("daemon", "Owned resource cleanup failed"),
    SessionCreated => ("session", "Session created"),
    SessionRenamed => ("session", "Session renamed"),
    SessionEnded => ("session", "Session ended"),
    SessionCleanupFailed => ("session", "Session cleanup failed"),
    ControllerAttached => ("session", "Controller attached"),
    ControllerDetached => ("session", "Controller detached"),
    ControllerTakenOver => ("session", "Controller taken over"),
    PrimaryEstablished => ("connection", "Primary connection established"),
    PrimaryClosed => ("connection", "Primary connection closed"),
    ConnectionStarted => ("connection", "Connection started"),
    ConnectionCompleted => ("connection", "Connection completed"),
    ConnectionFailed => ("connection", "Connection failed"),
    ConnectionStartup => ("connection_startup", "Initial connection stage"),
    ReconnectStarted => ("connection", "Reconnect started"),
    ReconnectCompleted => ("connection", "Reconnect completed"),
    ReconnectFailed => ("connection", "Reconnect failed"),
    RouteChanged => ("connection", "Connection route changed"),
    NetworkChanged => ("network", "Network state changed"),
    SyncChanged => ("terminal", "Terminal synchronization changed"),
    TerminalFailed => ("terminal", "Terminal operation failed"),
    TerminalLeaseLost => ("terminal", "Controller lease lost"),
    ViewStateChanged => ("terminal", "Terminal state changed"),
    PairOfferCreated => ("pairing", "Pairing offer created"),
    PairOfferFailed => ("pairing", "Pairing offer failed"),
    PairAcceptCommitted => ("pairing", "Outbound pairing committed"),
    PairAcceptFailed => ("pairing", "Pair acceptance failed"),
    InboundAuthorized => ("authorization", "Inbound authorization committed"),
    AuthorizationRevoked => ("authorization", "Inbound authorization revoked"),
    AuthorizationCleanup => ("authorization", "Revocation cleanup completed"),
    RequestFailed => ("service", "Request failed"),
    AdmissionRejected => ("service", "Admission rejected"),
    UploadStarted => ("upload", "Upload started"),
    UploadCompleted => ("upload", "Upload completed"),
    UploadCancelled => ("upload", "Upload cancelled"),
    UploadFailed => ("upload", "Upload failed"),
    UpdateStage => ("update", "Update stage changed"),
    UpdateCompleted => ("update", "Update completed"),
    AppStarted => ("android", "Application started"),
    AppLifecycle => ("android", "Application lifecycle changed"),
    NativeInitialized => ("android", "Native runtime initialized"),
    AppOperationFailed => ("android", "Application operation failed"),
    StorageFailed => ("android", "Application storage failed"),
    NotificationFailed => ("android", "Notification delivery failed"),
    InputFenceChanged => ("terminal", "Input admission changed"),
    ViewportChanged => ("terminal", "Viewport changed"),
    DetailEnabled => ("diagnostics", "Detailed diagnostics enabled"),
    DetailDisabled => ("diagnostics", "Detailed diagnostics disabled"),
    DetailExpired => ("diagnostics", "Detailed diagnostics expired"),
    RecordsLost => ("diagnostics", "Diagnostic records dropped or suppressed"),
    ExportCompleted => ("diagnostics", "Log export completed"),
    ExportFailed => ("diagnostics", "Log export failed"),
}

macro_rules! codes {
    ($name:ident, $doc:literal, $($v:ident),+ $(,)?) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $(#[doc = stringify!($v)] $v),+ }
    };
}
codes!(
    Outcome,
    "Observed outcome, independent of durable commit.",
    Started,
    Success,
    Failed,
    Cancelled,
    PartialCompletion,
    Unknown
);
codes!(
    Stage,
    "Fixed stage names shared by both platform adapters.",
    Starting,
    Preparing,
    Verified,
    Continuing,
    Stopping,
    Activating,
    Committed,
    AwaitingHandoff,
    Accepted,
    Initializing,
    Bound,
    Disabled,
    Foreground,
    Background,
    Saving,
    Loading,
    Connecting,
    Reconnecting,
    Active,
    Synchronizing,
    Ended,
    Closed,
    Detached,
    Failed,
    Cancelled,
    Direct,
    Relay,
    Unknown,
    Degraded,
    Ready,
    InputReady,
    InputBlocked,
    Snapshot,
    DeltaGap,
    Resize,
    Export,
    Notification
);
codes!(
    AddressService,
    "Content-free address service configuration/health.",
    Disabled,
    Configured,
    Degraded
);
codes!(
    Reason,
    "Safe non-error transition causes.",
    NaturalExit,
    ExplicitClose,
    DaemonStop,
    DriverFailure,
    TransportClosed,
    Detached,
    TakenOver,
    LeaseLost,
    Retry,
    Direct,
    Relay,
    Unknown,
    Degraded,
    Recovered,
    Shutdown,
    Replaced,
    Arbitration,
    StreamClosed,
    Cancelled,
    SyncRequired,
    RevisionGap,
    Promoted,
    Duplicate
);

codes!(
    FrontendFailure,
    "Closed platform failure categories without exception text.",
    TerminalIo,
    InvalidUsage,
    TerminalDriver,
    Storage,
    Notification,
    Export
);

codes!(
    NetworkFailure,
    "Closed network degradation categories.",
    EndpointBindFailed,
    EndpointClosed,
    HomeRelayUnavailable
);

/// One logical local operation; cloning preserves correlation and elapsed time.
#[derive(Clone, Debug)]
pub struct Operation {
    id: u64,
    started: Instant,
}
static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);
impl Default for Operation {
    fn default() -> Self {
        Self {
            id: NEXT_OPERATION.fetch_add(1, Ordering::Relaxed),
            started: Instant::now(),
        }
    }
}
impl Operation {
    /// Logging-only ordinal, scoped by the process instance.
    pub fn id(&self) -> u64 {
        self.id
    }
}

/// Producer event. Textual payloads and arbitrary error values have no API here.
#[derive(Clone, Debug)]
pub struct Event {
    pub(crate) kind: Kind,
    pub(crate) level: Level,
    pub(crate) fields: Fields,
}
impl Event {
    /// Creates a key event with INFO severity.
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            level: Level::Info,
            fields: Fields::default(),
        }
    }
    /// Overrides the severity; DEBUG routes exclusively to detail storage.
    pub fn level(mut self, level: Level) -> Self {
        self.level = level;
        self
    }
    /// Stable domain error category, never an error display/debug string.
    pub fn error(mut self, error: DomainErrorKind) -> Self {
        self.fields.category = Some(error.code().into());
        self
    }
    /// Authenticated release version, validated before it enters the schema.
    pub fn release_version(mut self, value: &str) -> Self {
        if value.len() <= 64 && semver::Version::parse(value.trim_start_matches('v')).is_ok() {
            self.fields.target_version = Some(value.into());
        }
        self
    }
    /// Whether the update owner accepted responsibility.
    pub fn accepted(mut self, value: bool) -> Self {
        self.fields.accepted = Some(value);
        self
    }
    /// Actual follow-up daemon startup result.
    pub fn daemon_started(mut self, value: bool) -> Self {
        self.fields.daemon_started = Some(value);
        self
    }
    /// Closed frontend failure category.
    pub fn frontend_error(mut self, error: FrontendFailure) -> Self {
        self.fields.category = Some(
            match error {
                FrontendFailure::TerminalIo => "terminal_io",
                FrontendFailure::InvalidUsage => "invalid_usage",
                FrontendFailure::TerminalDriver => "terminal_driver",
                FrontendFailure::Storage => "storage",
                FrontendFailure::Notification => "notification",
                FrontendFailure::Export => "export",
            }
            .into(),
        );
        self
    }
    /// Known local network degradation (without addresses or relay URLs).
    pub fn network_failure(mut self, failure: NetworkFailure) -> Self {
        self.fields.category = Some(
            match failure {
                NetworkFailure::EndpointBindFailed => "endpoint_bind_failed",
                NetworkFailure::EndpointClosed => "endpoint_closed",
                NetworkFailure::HomeRelayUnavailable => "home_relay_unavailable",
            }
            .into(),
        );
        self
    }
    /// Adds closed configuration/health states, never lookup addresses.
    pub fn address_services(
        mut self,
        publish: Option<AddressService>,
        lookup: Option<AddressService>,
    ) -> Self {
        self.fields.publish = publish;
        self.fields.lookup = lookup;
        self
    }
    /// Observed operation result.
    pub fn outcome(mut self, value: Outcome) -> Self {
        self.fields.outcome = Some(value);
        self
    }
    /// Fixed local stage.
    pub fn stage(mut self, value: Stage) -> Self {
        self.fields.stage = Some(value);
        self
    }
    /// Fixed lifecycle cause.
    pub fn reason(mut self, value: Reason) -> Self {
        self.fields.reason = Some(value);
        self
    }
    /// Existing typed initial-connection stage.
    pub fn startup(mut self, value: zterm_core::connection_progress::ConnectionStage) -> Self {
        self.fields.startup_stage = Some(value.description().0.into());
        self
    }
    /// Canonical Session identity, when known.
    pub fn session(mut self, value: SessionId) -> Self {
        self.fields.session_id = Some(value.to_string());
        self
    }
    /// Canonical attachment identity; never a resume capability.
    pub fn attachment(mut self, value: AttachmentId) -> Self {
        use std::fmt::Write;
        let mut text = String::with_capacity(32);
        for byte in value.as_bytes() {
            let _ = write!(text, "{byte:02x}");
        }
        self.fields.attachment_id = Some(text);
        self
    }
    /// Reuses operation correlation and its monotonic duration.
    pub fn operation(mut self, value: &Operation) -> Self {
        self.fields.operation_id = Some(value.id);
        self.fields.elapsed_ms = Some(
            value
                .started
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        );
        self
    }
    /// Logging-only connection ordinal.
    pub fn connection(mut self, value: u64) -> Self {
        self.fields.connection_id = Some(value);
        self
    }
    /// Durable commit is distinct from later cleanup success.
    pub fn committed(mut self, value: bool) -> Self {
        self.fields.committed = Some(value);
        self
    }
    /// Bounded aggregate count (not a payload).
    pub fn count(mut self, value: u64) -> Self {
        self.fields.count = Some(value);
        self
    }
    /// Observed transfer byte count; no filename/path/content.
    pub fn bytes(mut self, value: u64) -> Self {
        self.fields.bytes = Some(value);
        self
    }
    /// Observed synchronization/input epoch.
    pub fn epoch(mut self, value: u64) -> Self {
        self.fields.epoch = Some(value);
        self
    }
    /// Observed terminal dimensions.
    pub fn viewport(mut self, columns: u16, rows: u16) -> Self {
        self.fields.columns = Some(columns);
        self.fields.rows = Some(rows);
        self
    }
    /// Observed exit code (ordinary exits remain INFO).
    pub fn exit_code(mut self, value: Option<i32>) -> Self {
        self.fields.exit_code = value;
        self
    }
    /// Observed terminating signal.
    pub fn signal(mut self, value: Option<i32>) -> Self {
        self.fields.signal = value;
        self
    }
}

/// Only safe optional event data is admitted by the schema decoder.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fields {
    /// Address publisher configuration/health, without addresses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish: Option<AddressService>,
    /// Address lookup configuration/health, without addresses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookup: Option<AddressService>,
    /// Authenticated release target, never an unverified user-supplied URL/tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
    /// Whether update handoff transferred responsibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted: Option<bool>,
    /// Follow-up daemon startup outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_started: Option<bool>,
    /// Process-local operation ordinal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<u64>,
    /// Process-local connection ordinal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<u64>,
    /// Canonical public Session identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Canonical attachment identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    /// Monotonic operation duration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// Fixed stage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
    /// Fixed first-screen stage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_stage: Option<String>,
    /// Domain error code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Lifecycle reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    /// Actual result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// Whether the owner's durable commit occurred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committed: Option<bool>,
    /// Aggregated record/event count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    /// Transfer byte count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Input/synchronization epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    /// Viewport columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<u16>,
    /// Viewport rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    /// Process exit code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Terminating signal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
}

/// Validated version-one record. Unknown fields fail closed on import/export.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// Schema version.
    pub schema: u8,
    /// UTC RFC3339 time with millisecond precision.
    pub timestamp: String,
    /// Event severity.
    pub level: Level,
    /// Stable owner.
    pub component: String,
    /// Closed event kind.
    pub event: Kind,
    /// Fixed English message.
    pub message: String,
    /// Non-secret process-instance identifier.
    pub process_instance: String,
    /// Local OS process identifier.
    pub pid: u32,
    /// Application build version.
    pub version: String,
    /// Compile-time operating system.
    pub platform: String,
    /// Sequence scoped to this recorder instance.
    pub sequence: u64,
    /// Safe event data; nested to keep deny_unknown_fields effective.
    pub fields: Fields,
}
impl Record {
    pub(crate) fn new(event: Event, instance: &str, sequence: u64) -> Self {
        let now = OffsetDateTime::now_utc();
        let now = now
            .replace_nanosecond(now.millisecond() as u32 * 1_000_000)
            .unwrap_or(now);
        Self {
            schema: 1,
            timestamp: now.format(&Rfc3339).unwrap_or_default(),
            level: event.level,
            component: event.kind.component().into(),
            event: event.kind,
            message: event.kind.message().into(),
            process_instance: instance.into(),
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").into(),
            platform: std::env::consts::OS.into(),
            sequence,
            fields: event.fields,
        }
    }
    /// Encodes one complete bounded JSONL record.
    pub fn encode(&self) -> Option<Vec<u8>> {
        let mut bytes = serde_json::to_vec(self).ok()?;
        bytes.push(b'\n');
        (bytes.len() <= MAX_RECORD_BYTES).then_some(bytes)
    }
    /// Decodes only the known, content-free schema.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_RECORD_BYTES {
            return None;
        }
        let record: Self = serde_json::from_slice(bytes).ok()?;
        if record.schema != 1
            || record.component != record.event.component()
            || record.message != record.event.message()
            || record.process_instance.len() > 64
            || !record
                .process_instance
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b'-')
            || record.version.len() > 64
            || !record
                .version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
            || !matches!(
                record.platform.as_str(),
                "android" | "linux" | "macos" | "windows" | "ios" | "freebsd"
            )
            || OffsetDateTime::parse(&record.timestamp, &Rfc3339).is_err()
        {
            return None;
        }
        for id in [&record.fields.session_id, &record.fields.attachment_id]
            .into_iter()
            .flatten()
        {
            if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
        }
        if record.fields.category.as_deref().is_some_and(|code| {
            DomainErrorKind::from_code(code).is_none()
                && !matches!(
                    code,
                    "terminal_io"
                        | "invalid_usage"
                        | "terminal_driver"
                        | "storage"
                        | "notification"
                        | "export"
                        | "endpoint_bind_failed"
                        | "endpoint_closed"
                        | "home_relay_unavailable"
                )
        }) {
            return None;
        }
        if let Some(stage) = &record.fields.startup_stage
            && ![
                "starting",
                "initializing_terminal",
                "terminal_initialized",
                "checking_local_service",
                "local_service_ready",
                "resolving_target",
                "target_resolved",
                "opening_local_channel",
                "opening_remote_channel",
                "checking_connection",
                "reusing_connection",
                "looking_up_address",
                "connecting_securely",
                "secure_connection_ready",
                "checking_protocol_and_access",
                "protocol_and_access_ready",
                "selecting_connection",
                "opening_session_channel",
                "session_channel_ready",
                "retrying_connection",
                "creating_session",
                "session_created",
                "requesting_session",
                "receiving_terminal_state",
                "terminal_state_received",
                "displaying_terminal",
                "synchronizing_terminal",
                "terminal_ready",
                "cancelling",
                "session_ended",
                "cancelled",
                "failed",
            ]
            .contains(&stage.as_str())
        {
            return None;
        }
        if record
            .fields
            .target_version
            .as_deref()
            .is_some_and(|version| {
                version.len() > 64
                    || semver::Version::parse(version.trim_start_matches('v')).is_err()
            })
        {
            return None;
        }
        Some(record)
    }
    /// Human-readable one-line rendering without ANSI or payload text.
    pub fn render(&self) -> String {
        let fields = serde_json::to_value(&self.fields).unwrap_or_default();
        let mut line = format!(
            "{} {:?} {} {} pid={} process={} sequence={}",
            self.timestamp,
            self.level,
            self.component,
            self.message,
            self.pid,
            self.process_instance,
            self.sequence
        );
        if let Some(fields) = fields.as_object() {
            use std::fmt::Write;
            for (key, value) in fields {
                let _ = write!(
                    line,
                    " {key}={}",
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned)
                );
            }
        }
        line
    }
}
