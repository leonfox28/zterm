//! Nonblocking producers and a single bounded sink worker.

use crate::{DETAIL_SECONDS, Event, Kind, Level, QUEUE_BYTES, QUEUE_RECORDS, Record};
use serde::{Deserialize, Serialize};
use std::io;
use std::sync::{
    Arc, Condvar, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc::{self, Receiver, SyncSender},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Versioned finite detail interval, stored separately from product configuration.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Control {
    /// Control schema.
    pub schema: u8,
    /// UTC Unix milliseconds of explicit enable/renewal.
    pub started_ms: i64,
    /// UTC Unix milliseconds of expiry; zero means disabled.
    pub deadline_ms: i64,
}
impl Control {
    /// Creates a fresh fifteen-minute interval at the supplied wall time.
    pub fn enabled(now_ms: i64) -> Self {
        Self {
            schema: 1,
            started_ms: now_ms,
            deadline_ms: now_ms.saturating_add(DETAIL_SECONDS * 1000),
        }
    }
    /// Creates an explicit OFF marker.
    pub fn disabled() -> Self {
        Self {
            schema: 1,
            started_ms: 0,
            deadline_ms: 0,
        }
    }
    /// Validates the interval and reports the remaining wall-clock duration.
    pub fn remaining_ms(self, now_ms: i64) -> u64 {
        if self.schema != 1
            || self.started_ms <= 0
            || now_ms < self.started_ms
            || self.deadline_ms.checked_sub(self.started_ms) != Some(DETAIL_SECONDS * 1000)
        {
            return 0;
        }
        self.deadline_ms.saturating_sub(now_ms).max(0) as u64
    }
}
/// Current UTC Unix milliseconds, independent of a runtime or UI thread.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

/// Platform storage boundary, called only from the recorder worker.
pub trait Sink: Send + Sync + 'static {
    /// Append one complete JSONL record to the selected lane.
    fn append(&self, detail: bool, bytes: &[u8]) -> io::Result<()>;
    /// Refresh the persisted interval even when no events arrive.
    fn control(&self) -> io::Result<Control>;
    /// Explicit write durability barrier used by updater/lifecycle owners.
    fn flush(&self) -> io::Result<()> {
        Ok(())
    }
}

struct Lane {
    sender: SyncSender<Vec<u8>>,
    bytes: AtomicUsize,
}
#[derive(Default)]
struct Admission {
    rate: Option<(Instant, usize)>,
    summaries: Vec<(Kind, Option<String>, Instant)>,
}
#[derive(Default)]
struct FlushState {
    requested: u64,
    completed: u64,
    ok: bool,
}
struct Shared {
    lanes: [Lane; 2],
    sequence: AtomicU64,
    admission: Mutex<Admission>,
    control: Mutex<FlushState>,
    stopped: AtomicBool,
    wake: Condvar,
    detail: AtomicBool,
    origin: Instant,
    detail_until_ms: AtomicU64,
    dropped: AtomicU64,
    suppressed: AtomicU64,
    instance: String,
}
struct Owner(Arc<Shared>);
impl Drop for Owner {
    fn drop(&mut self) {
        self.0.stopped.store(true, Ordering::Release);
        self.0.wake.notify_all();
    }
}
/// Cloneable recorder. Dropping the last handle requests draining without blocking.
#[derive(Clone, Default)]
pub struct Recorder(Option<Arc<Owner>>);
impl Recorder {
    /// Starts one worker; sink callbacks never run on producers.
    pub fn new(sink: Arc<dyn Sink>) -> io::Result<Self> {
        static INSTANCES: AtomicU64 = AtomicU64::new(1);
        let (key_sender, key_receiver) = mpsc::sync_channel(QUEUE_RECORDS);
        let (detail_sender, detail_receiver) = mpsc::sync_channel(QUEUE_RECORDS);
        let shared = Arc::new(Shared {
            lanes: [key_sender, detail_sender].map(|sender| Lane {
                sender,
                bytes: AtomicUsize::new(0),
            }),
            sequence: AtomicU64::new(0),
            admission: Mutex::new(Admission::default()),
            control: Mutex::new(FlushState::default()),
            stopped: AtomicBool::new(false),
            wake: Condvar::new(),
            detail: AtomicBool::new(false),
            origin: Instant::now(),
            detail_until_ms: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            suppressed: AtomicU64::new(0),
            instance: format!(
                "{:x}-{:x}-{:x}",
                now_ms(),
                std::process::id(),
                INSTANCES.fetch_add(1, Ordering::Relaxed)
            ),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("zterm-diagnostics".into())
            .spawn(move || run(worker, [key_receiver, detail_receiver], sink))?;
        Ok(Self(Some(Arc::new(Owner(shared)))))
    }
    /// Cheap admission check before constructing detailed observations.
    pub fn detail_enabled(&self) -> bool {
        self.0.as_ref().is_some_and(|owner| {
            owner.0.detail.load(Ordering::Relaxed)
                && owner.0.origin.elapsed().as_millis()
                    < u128::from(owner.0.detail_until_ms.load(Ordering::Relaxed))
        })
    }
    /// Enqueues without waiting for a lock or performing filesystem I/O.
    pub fn record(&self, event: Event) -> bool {
        let Some(owner) = &self.0 else {
            return false;
        };
        let shared = &owner.0;
        let detail = event.level == Level::Debug;
        if detail && !self.detail_enabled() {
            return false;
        }
        if !admit(shared, &event, detail) {
            return false;
        }
        let sequence = shared.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let Some(bytes) = Record::new(event, &shared.instance, sequence).encode() else {
            shared.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        let lane = &shared.lanes[usize::from(detail)];
        let length = bytes.len();
        if lane
            .bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used + length <= QUEUE_BYTES).then_some(used + length)
            })
            .is_err()
        {
            shared.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        if lane.sender.try_send(bytes).is_err() {
            lane.bytes.fetch_sub(length, Ordering::AcqRel);
            shared.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        shared.wake.notify_all();
        true
    }
    /// Waits at most the supplied duration for a durability barrier. Never use on UI/terminal actors.
    pub fn flush(&self, timeout: Duration) -> bool {
        let Some(owner) = &self.0 else {
            return true;
        };
        let shared = &owner.0;
        let deadline = Instant::now() + timeout;
        let mut control = loop {
            match shared.control.try_lock() {
                Ok(control) => break control,
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        };
        control.requested += 1;
        let requested = control.requested;
        shared.wake.notify_all();
        while control.completed < requested {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let (next, timed) = shared
                .wake
                .wait_timeout(control, remaining)
                .unwrap_or_else(|e| e.into_inner());
            control = next;
            if timed.timed_out() {
                return false;
            }
        }
        control.ok
    }
    /// Number of records still unreported due to loss or suppression.
    pub fn lost(&self) -> u64 {
        self.0.as_ref().map_or(0, |o| {
            o.0.dropped
                .load(Ordering::Relaxed)
                .saturating_add(o.0.suppressed.load(Ordering::Relaxed))
        })
    }
}

// Only explicitly summarized events share admission state. Ordinary key events
// cannot be lost merely because a worker or another producer is holding a mutex.
fn admit(shared: &Shared, event: &Event, detail: bool) -> bool {
    let summary = matches!(
        event.kind,
        Kind::AdmissionRejected | Kind::ListenerFailed | Kind::RequestFailed
    ) || detail
        && matches!(
            event.kind,
            Kind::SyncChanged | Kind::ViewportChanged | Kind::InputFenceChanged
        );
    if !detail && !summary {
        return true;
    }
    let Ok(mut admission) = shared.admission.try_lock() else {
        shared.suppressed.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    if detail {
        let (start, count) = admission.rate.get_or_insert((Instant::now(), 0));
        if start.elapsed() >= Duration::from_secs(1) {
            *start = Instant::now();
            *count = 0;
        }
        if *count >= 100 {
            shared.suppressed.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        *count += 1;
    }
    if summary {
        let key = if detail {
            event.fields.session_id.clone()
        } else {
            event.fields.category.clone()
        };
        let now = Instant::now();
        admission
            .summaries
            .retain(|(_, _, time)| now.duration_since(*time) < Duration::from_secs(1));
        if admission.summaries.len() >= 128
            || admission
                .summaries
                .iter()
                .any(|(kind, category, _)| *kind == event.kind && *category == key)
        {
            shared.suppressed.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        admission.summaries.push((event.kind, key, now));
    }
    true
}

#[derive(Default)]
struct DetailInterval {
    marker: Control,
    deadline_ms: u64,
}
impl DetailInterval {
    fn observe(
        &mut self,
        observed: Option<Control>,
        wall_ms: i64,
        monotonic_ms: u64,
    ) -> (bool, Kind) {
        if self.marker.remaining_ms(wall_ms) == 0 || monotonic_ms >= self.deadline_ms {
            self.deadline_ms = 0;
        }
        let Some(marker) = observed else {
            return (false, Kind::DetailDisabled);
        };
        let remaining = marker.remaining_ms(wall_ms);
        // OFF, malformed markers and read errors must not forget an interval's
        // original cap. Restoring the same marker after clock rollback is not
        // an explicit renewal; only a new valid enable marker grants more time.
        if remaining > 0 && marker != self.marker {
            self.marker = marker;
            self.deadline_ms = monotonic_ms.saturating_add(remaining);
        }
        let enabled = remaining > 0 && monotonic_ms < self.deadline_ms;
        let kind = if enabled {
            Kind::DetailEnabled
        } else if marker.deadline_ms > 0 {
            Kind::DetailExpired
        } else {
            Kind::DetailDisabled
        };
        (enabled, kind)
    }
}

fn run(shared: Arc<Shared>, receivers: [Receiver<Vec<u8>>; 2], sink: Arc<dyn Sink>) {
    let mut last_poll = Instant::now() - Duration::from_secs(2);
    let mut interval = DetailInterval::default();
    let mut write_ok = true;
    let mut last_loss_report = Instant::now() - Duration::from_secs(2);
    loop {
        if last_poll.elapsed() >= Duration::from_secs(1) {
            let (enabled, kind) = interval.observe(
                sink.control().ok(),
                now_ms(),
                shared
                    .origin
                    .elapsed()
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
            );
            shared
                .detail_until_ms
                .store(interval.deadline_ms, Ordering::Relaxed);
            let was_enabled = shared.detail.swap(enabled, Ordering::Relaxed);
            if enabled != was_enabled {
                write_ok &= write_internal(&shared, &*sink, Event::new(kind));
            }
            last_poll = Instant::now();
        }
        let item = receivers.iter().enumerate().find_map(|(index, receiver)| {
            receiver.try_recv().ok().map(|bytes| {
                shared.lanes[index]
                    .bytes
                    .fetch_sub(bytes.len(), Ordering::AcqRel);
                (index == 1, bytes)
            })
        });
        if let Some((detail, bytes)) = item {
            if sink.append(detail, &bytes).is_err() {
                shared.dropped.fetch_add(1, Ordering::Relaxed);
                write_ok = false;
            } else if last_loss_report.elapsed() >= Duration::from_secs(1) {
                let lost = shared
                    .dropped
                    .swap(0, Ordering::Relaxed)
                    .saturating_add(shared.suppressed.swap(0, Ordering::Relaxed));
                if lost > 0
                    && !write_internal(
                        &shared,
                        &*sink,
                        Event::new(Kind::RecordsLost).level(Level::Warn).count(lost),
                    )
                {
                    shared.dropped.fetch_add(lost, Ordering::Relaxed);
                }
                last_loss_report = Instant::now();
            }
            continue;
        }
        let mut control = shared.control.lock().unwrap_or_else(|e| e.into_inner());
        // Byte reservations include producers between reservation and try_send.
        // Do not acknowledge a barrier while an admitted record is still pending.
        if shared
            .lanes
            .iter()
            .any(|lane| lane.bytes.load(Ordering::Acquire) != 0)
        {
            continue;
        }
        if control.completed < control.requested {
            let generation = control.requested;
            drop(control);
            let ok = sink.flush().is_ok() && write_ok;
            control = shared.control.lock().unwrap_or_else(|e| e.into_inner());
            control.completed = generation;
            control.ok = ok;
            write_ok = true;
            shared.wake.notify_all();
        }
        if shared.stopped.load(Ordering::Acquire) {
            drop(control);
            let _ = sink.flush();
            return;
        }
        // Producers don't acquire the sleep mutex. The bounded timeout also
        // handles a notification racing with entry into this wait.
        let _ = shared
            .wake
            .wait_timeout(control, Duration::from_millis(100));
    }
}
fn write_internal(shared: &Shared, sink: &dyn Sink, event: Event) -> bool {
    let sequence = shared.sequence.fetch_add(1, Ordering::Relaxed) + 1;
    let ok = Record::new(event, &shared.instance, sequence)
        .encode()
        .is_some_and(|bytes| sink.append(false, &bytes).is_ok());
    if !ok {
        shared.dropped.fetch_add(1, Ordering::Relaxed);
    }
    ok
}

static GLOBAL: OnceLock<Recorder> = OnceLock::new();
/// Installs the process composition exactly once. Tests can use injected recorders.
pub fn install(recorder: Recorder) -> bool {
    GLOBAL.set(recorder).is_ok()
}
/// Emits through the installed composition; otherwise a no-op.
pub fn record(event: Event) {
    if let Some(recorder) = GLOBAL.get() {
        recorder.record(event);
    }
}
/// Checks the process-wide detail gate.
pub fn detail_enabled() -> bool {
    GLOBAL.get().is_some_and(Recorder::detail_enabled)
}
/// Flushes the process composition with a bounded wait.
pub fn flush(timeout: Duration) -> bool {
    GLOBAL.get().is_none_or(|recorder| recorder.flush(timeout))
}

/// Clones the installed process composition without creating a worker.
pub fn global_recorder() -> Option<Recorder> {
    GLOBAL.get().cloned()
}

#[cfg(test)]
mod interval_tests {
    use super::*;

    #[test]
    fn restoring_a_marker_after_control_failure_or_clock_rollback_cannot_renew_it() {
        let marker = Control::enabled(1_000_000);
        let mut interval = DetailInterval::default();
        assert!(interval.observe(Some(marker), 1_899_000, 0).0);
        assert_eq!(interval.deadline_ms, 1_000);
        for observed in [None, Some(Control::disabled()), Some(Control::default())] {
            assert!(!interval.observe(observed, 1_899_100, 100).0);
            assert_eq!(interval.deadline_ms, 1_000);
        }
        assert!(interval.observe(Some(marker), 1_898_000, 400).0);
        assert_eq!(interval.deadline_ms, 1_000);
        assert_eq!(
            interval.observe(Some(marker), 1_898_500, 1_001),
            (false, Kind::DetailExpired)
        );
        let renewed = Control::enabled(1_898_500);
        assert!(interval.observe(Some(renewed), 1_898_500, 1_002).0);
        assert!(!interval.observe(Some(renewed), 2_800_000, 1_003).0);
        assert!(!interval.observe(Some(renewed), 1_899_000, 1_004).0);
    }
}
