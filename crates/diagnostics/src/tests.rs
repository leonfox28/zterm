use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Memory {
    records: Mutex<Vec<(bool, Vec<u8>)>>,
    control: Mutex<Control>,
    fail: AtomicBool,
}
impl Sink for Memory {
    fn append(&self, detail: bool, bytes: &[u8]) -> std::io::Result<()> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(std::io::ErrorKind::StorageFull.into());
        }
        self.records
            .lock()
            .expect("memory sink")
            .push((detail, bytes.into()));
        Ok(())
    }
    fn control(&self) -> std::io::Result<Control> {
        Ok(*self.control.lock().expect("control"))
    }
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < deadline, "worker deadline");
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn finite_control_rejects_rollback_malformed_and_expired_intervals() {
    let now = 1_000_000;
    let control = Control::enabled(now);
    assert_eq!(control.remaining_ms(now), 900_000);
    assert_eq!(control.remaining_ms(now - 1), 0);
    assert_eq!(control.remaining_ms(now + 900_000), 0);
    assert_eq!(
        Control {
            deadline_ms: i64::MAX,
            ..control
        }
        .remaining_ms(now),
        0
    );
    assert_eq!(
        Control {
            schema: 99,
            ..control
        }
        .remaining_ms(now),
        0
    );
    assert_eq!(Control::disabled().remaining_ms(now), 0);
}
#[test]
fn worker_routes_detail_only_during_explicit_interval_and_preserves_safe_correlation() {
    let sink = Arc::new(Memory::default());
    let recorder = Recorder::new(sink.clone()).expect("worker");
    let operation = Operation::default();
    recorder.record(Event::new(Kind::ConnectionStarted).operation(&operation));
    recorder.record(Event::new(Kind::SyncChanged).level(Level::Debug));
    assert!(recorder.flush(Duration::from_secs(1)));
    assert!(
        sink.records
            .lock()
            .expect("records")
            .iter()
            .all(|(detail, _)| !detail)
    );
    *sink.control.lock().expect("control") = Control::enabled(now_ms());
    wait(|| recorder.detail_enabled());
    recorder.record(
        Event::new(Kind::SyncChanged)
            .level(Level::Debug)
            .operation(&operation),
    );
    recorder.record(
        Event::new(Kind::ConnectionFailed)
            .level(Level::Warn)
            .error(zterm_core::DomainErrorKind::Unauthorized)
            .operation(&operation),
    );
    assert!(recorder.flush(Duration::from_secs(1)));
    let records = sink.records.lock().expect("records");
    assert_eq!(records.iter().filter(|(detail, _)| *detail).count(), 1);
    let events: Vec<_> = records
        .iter()
        .map(|(_, bytes)| Record::decode(bytes).expect("known record"))
        .collect();
    let failed = events
        .iter()
        .find(|r| r.event == Kind::ConnectionFailed)
        .expect("outcome");
    assert_eq!(failed.fields.operation_id, Some(operation.id()));
    assert_eq!(failed.fields.category.as_deref(), Some("unauthorized"));
    assert!(failed.fields.elapsed_ms.is_some());
    drop(records);
    *sink.control.lock().expect("control") = Control::enabled(now_ms() - 900_001);
    wait(|| !recorder.detail_enabled());
    recorder.record(Event::new(Kind::SyncChanged).level(Level::Debug));
    assert!(recorder.flush(Duration::from_secs(1)));
    assert_eq!(
        sink.records
            .lock()
            .expect("records")
            .iter()
            .filter(|(detail, _)| *detail)
            .count(),
        1
    );
}
#[test]
fn import_rejects_payload_fields_forged_messages_unknown_codes_and_oversize() {
    let record = Record::new(Event::new(Kind::UploadFailed), "123-ab", 1);
    let mut json = serde_json::to_value(&record).expect("record");
    json["fields"]["filename"] = "SECRET_SENTINEL".into();
    assert!(Record::decode(&serde_json::to_vec(&json).expect("json")).is_none());
    let mut json = serde_json::to_value(&record).expect("record");
    json["message"] = "TERMINAL_SENTINEL".into();
    assert!(Record::decode(&serde_json::to_vec(&json).expect("json")).is_none());
    let mut json = serde_json::to_value(&record).expect("record");
    json["fields"]["category"] = "CREDENTIAL_SENTINEL".into();
    assert!(Record::decode(&serde_json::to_vec(&json).expect("json")).is_none());
    assert!(Record::decode(&vec![b'x'; MAX_RECORD_BYTES + 1]).is_none());
}
#[test]
fn sink_failure_does_not_block_producers_and_loss_is_reported_after_recovery() {
    let sink = Arc::new(Memory::default());
    sink.fail.store(true, Ordering::Relaxed);
    let recorder = Recorder::new(sink.clone()).expect("worker");
    recorder.record(Event::new(Kind::AppStarted));
    assert!(!recorder.flush(Duration::from_secs(1)));
    assert!(recorder.lost() > 0);
    sink.fail.store(false, Ordering::Relaxed);
    recorder.record(Event::new(Kind::NativeInitialized));
    assert!(recorder.flush(Duration::from_secs(1)));
    assert!(
        sink.records
            .lock()
            .expect("records")
            .iter()
            .any(|(_, bytes)| Record::decode(bytes).is_some_and(
                |r| r.event == Kind::RecordsLost && r.fields.count.is_some_and(|n| n > 0)
            ))
    );
}

#[test]
fn blocked_sink_keeps_producers_bounded_and_detail_expires_without_worker_polling() {
    struct Blocked {
        memory: Memory,
        entered: AtomicBool,
        released: Mutex<bool>,
        wake: std::sync::Condvar,
    }
    impl Sink for Blocked {
        fn append(&self, detail: bool, bytes: &[u8]) -> std::io::Result<()> {
            self.entered.store(true, Ordering::Release);
            let guard = self.released.lock().expect("gate");
            let _guard = self
                .wake
                .wait_while(guard, |released| !*released)
                .expect("gate");
            self.memory.append(detail, bytes)
        }
        fn control(&self) -> std::io::Result<Control> {
            self.memory.control()
        }
    }
    let sink = Arc::new(Blocked {
        memory: Memory::default(),
        entered: AtomicBool::new(false),
        released: Mutex::new(false),
        wake: std::sync::Condvar::new(),
    });
    *sink.memory.control.lock().expect("interval") = Control::enabled(now_ms() - 898_800);
    let recorder = Recorder::new(sink.clone()).expect("recorder");
    wait(|| sink.entered.load(Ordering::Acquire));
    let start = Instant::now();
    for _ in 0..QUEUE_RECORDS {
        assert!(recorder.record(Event::new(Kind::ConnectionStarted)));
    }
    assert!(
        !recorder.record(Event::new(Kind::ConnectionStarted)),
        "entry bound"
    );
    assert!(
        recorder.record(Event::new(Kind::ViewportChanged).level(Level::Debug)),
        "independent detail capacity"
    );
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "producers never wait for sink"
    );
    assert!(
        !recorder.flush(Duration::from_millis(20)),
        "flush has a deadline"
    );
    wait(|| !recorder.detail_enabled());
    assert!(
        !recorder.record(Event::new(Kind::RouteChanged).level(Level::Debug)),
        "expiry does not depend on a writable sink"
    );
    *sink.released.lock().expect("release") = true;
    sink.wake.notify_all();
    assert!(recorder.flush(Duration::from_secs(2)));
    let records = sink.memory.records.lock().expect("records");
    assert_eq!(
        records
            .iter()
            .filter(|(_, bytes)| Record::decode(bytes)
                .is_some_and(|r| r.event == Kind::ConnectionStarted))
            .count(),
        QUEUE_RECORDS
    );
    assert!(
        records
            .iter()
            .any(|(_, bytes)| Record::decode(bytes).is_some_and(|r| r.event == Kind::RecordsLost))
    );
}

#[test]
fn merged_tail_orders_rfc3339_fractional_seconds_by_time_not_text() {
    use std::io::Write;
    let mut snapshots = Vec::new();
    for (detail, timestamps) in [
        (false, ["2026-09-27T00:00:00.9Z", "2026-09-27T00:00:00.01Z"]),
        (true, ["2026-09-27T00:00:00.11Z", "2026-09-27T00:00:00Z"]),
    ] {
        let mut file = tempfile::tempfile().expect("snapshot");
        for timestamp in timestamps {
            let mut record = Record::new(Event::new(Kind::ConnectionStarted), "abc-123", 1);
            record.timestamp = timestamp.into();
            file.write_all(&record.encode().expect("known record"))
                .expect("record");
        }
        snapshots.push(Snapshot {
            length: file.metadata().expect("length").len(),
            file,
            start: 0,
            detail,
        });
    }
    let result = tail(
        &mut snapshots,
        &Filter {
            lines: 4,
            include_debug: true,
            json: true,
            ..Default::default()
        },
    )
    .expect("merged tail");
    let timestamps: Vec<_> = result
        .lines
        .iter()
        .map(|line| Record::decode(line.as_bytes()).expect("record").timestamp)
        .collect();
    assert_eq!(
        timestamps,
        [
            "2026-09-27T00:00:00Z",
            "2026-09-27T00:00:00.01Z",
            "2026-09-27T00:00:00.11Z",
            "2026-09-27T00:00:00.9Z"
        ]
    );
}
