//! Bounded, optional observations; never an operation or connection state owner.

use std::{
    collections::VecDeque,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::watch;
use zterm_core::{DomainErrorKind, connection_progress::ConnectionStage};

/// Maximum retained stage records for a slow screen or local progress reader.
pub const CONNECTION_PROGRESS_CAPACITY: usize = 64;

/// A fixed stage and optional local, typed failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgressEvent {
    /// Observed stage.
    pub stage: ConnectionStage,
    /// Stable failure category; never a source error string.
    pub failure: Option<ProgressFailure>,
}

/// Safe failure categories from the operation or physical frontend boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressFailure {
    /// A typed operation failure.
    Domain(DomainErrorKind),
    /// Physical terminal setup, input or presentation failed.
    TerminalIo,
    /// The interactive invocation was invalid.
    InvalidUsage,
    /// The retained terminal driver failed.
    TerminalDriver,
}

impl ProgressFailure {
    /// Stable diagnostic value without a source error string.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Domain(kind) => kind.code(),
            Self::TerminalIo => "terminal_io",
            Self::InvalidUsage => "invalid_usage",
            Self::TerminalDriver => "terminal_driver",
        }
    }
}

impl From<DomainErrorKind> for ProgressFailure {
    fn from(kind: DomainErrorKind) -> Self {
        Self::Domain(kind)
    }
}

/// Bounded journal snapshots preserve bursts without queueing operation work.
#[derive(Clone, Debug, Default)]
pub struct ProgressHistory {
    sequence: u64,
    entries: VecDeque<(u64, ProgressEvent)>,
}

impl ProgressHistory {
    /// Appends one observation, coalescing identical consecutive notifications.
    pub fn record(&mut self, event: ProgressEvent) {
        if self
            .entries
            .back()
            .is_some_and(|(_, previous)| *previous == event)
        {
            return;
        }
        self.sequence = self.sequence.saturating_add(1);
        if self.entries.len() == CONNECTION_PROGRESS_CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back((self.sequence, event));
    }

    /// Drops a previous demand cycle's presentation without reusing sequence IDs.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Retained records newer than a reader's watermark, in emission order.
    pub fn after(&self, sequence: u64) -> impl Iterator<Item = (u64, ProgressEvent)> + '_ {
        self.entries
            .iter()
            .copied()
            .filter(move |(id, _)| *id > sequence)
    }
}

struct ObserverInner {
    active: AtomicBool,
    report: Box<dyn Fn(ProgressEvent) + Send + Sync>,
}

/// Optional synchronous observer, explicitly retired at the initial Active fence.
#[derive(Clone, Default)]
pub struct ProgressObserver(Option<Arc<ObserverInner>>);

impl fmt::Debug for ProgressObserver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProgressObserver")
            .field("active", &self.is_active())
            .finish()
    }
}

impl ProgressObserver {
    /// A constant no-op observer for callers without startup presentation.
    #[must_use]
    pub const fn disabled() -> Self {
        Self(None)
    }
    /// Installs a content-free sink. The sink must not drive or fail operations.
    pub fn new(report: impl Fn(ProgressEvent) + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(ObserverInner {
            active: AtomicBool::new(true),
            report: Box::new(report),
        })))
    }

    /// Creates a coalescing screen channel and an optional persistent event sink.
    pub fn channel(
        sink: impl Fn(ProgressEvent) + Send + Sync + 'static,
    ) -> (Self, watch::Receiver<ProgressHistory>) {
        let (sender, receiver) = watch::channel(ProgressHistory::default());
        (
            Self::new(move |event| {
                sink(event);
                sender.send_modify(|history| history.record(event));
            }),
            receiver,
        )
    }

    /// Reports an observed boundary without waiting for a screen reader.
    pub fn report(&self, stage: ConnectionStage) {
        self.emit(ProgressEvent {
            stage,
            failure: None,
        });
    }

    /// Records the startup failure through its stable domain category.
    pub fn fail(&self, failure: impl Into<ProgressFailure>) {
        let failure = failure.into();
        self.emit(ProgressEvent {
            stage: if failure == ProgressFailure::Domain(DomainErrorKind::Cancelled) {
                ConnectionStage::Cancelled
            } else {
                ConnectionStage::Failed
            },
            failure: Some(failure),
        });
    }

    /// Forwards an already typed event.
    pub fn emit(&self, event: ProgressEvent) {
        if let Some(inner) = &self.0
            && inner.active.load(Ordering::Acquire)
        {
            (inner.report)(event);
        }
    }

    /// Whether startup still accepts observations.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|inner| inner.active.load(Ordering::Acquire))
    }

    /// Retires every clone so a retained connector cannot log reconnect as startup.
    pub fn stop(&self) {
        if let Some(inner) = &self.0 {
            inner.active.store(false, Ordering::Release);
        }
    }
}

use crate::{
    error::ClientError as DaemonError,
    protocol::{malformed, protocol_error},
};
use zterm_proto::{DecodedFrame, WireKind, connection_stage_from_message, v2};

/// The only same-UID stage validator, shared by tunnel and unary readers.
pub struct LocalProgressDecoder {
    request_id: u64,
    observer: ProgressObserver,
    count: usize,
}

impl LocalProgressDecoder {
    /// Binds a reader to the exact same-UID request and optional observer.
    pub fn new(request_id: u64, observer: ProgressObserver) -> Self {
        Self {
            request_id,
            observer,
            count: 0,
        }
    }

    /// Consumes only a validated progress frame, leaving final replies to their owner.
    pub fn consume(&mut self, frame: &DecodedFrame) -> Result<bool, DaemonError> {
        if frame.kind != WireKind::LocalConnectionProgress {
            return Ok(false);
        }
        if !self.observer.is_active()
            || frame.request_id != self.request_id
            || frame.deadline_ms != 0
        {
            return Err(malformed(
                "unexpected local connection progress correlation",
            ));
        }
        self.count += 1;
        if self.count > CONNECTION_PROGRESS_CAPACITY {
            return Err(malformed("local connection progress exceeded its bound"));
        }
        let message: v2::LocalConnectionProgress = frame
            .decode_message(WireKind::LocalConnectionProgress)
            .map_err(protocol_error)?;
        self.observer
            .report(connection_stage_from_message(message).map_err(protocol_error)?);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use zterm_proto::{FrameDecoder, encode_message};

    #[test]
    fn progress_journal_retains_bursts_bounds_history_and_retires_all_clones() {
        let persisted = Arc::new(Mutex::new(Vec::new()));
        let sink = persisted.clone();
        let (observer, history) = ProgressObserver::channel(move |event| {
            sink.lock().expect("progress sink lock").push(event)
        });
        let retained = observer.clone();
        for _ in 0..40 {
            observer.report(ConnectionStage::LookingUpAddress);
            observer.report(ConnectionStage::RetryingConnection);
        }
        let records = history.borrow().after(0).collect::<Vec<_>>();
        assert_eq!(records.len(), CONNECTION_PROGRESS_CAPACITY);
        assert_eq!(records[0].0, 17);
        assert_eq!(records.last().expect("retained progress").0, 80);
        assert!(records.windows(2).all(|pair| pair[0].0 + 1 == pair[1].0));
        observer.fail(DomainErrorKind::Unauthorized);
        observer.stop();
        retained.report(ConnectionStage::TerminalReady);
        assert_eq!(persisted.lock().expect("persisted progress").len(), 81);
        assert_eq!(
            history
                .borrow()
                .after(80)
                .next()
                .expect("failure observation")
                .1
                .failure,
            Some(ProgressFailure::Domain(DomainErrorKind::Unauthorized))
        );
    }

    fn progress_frame(request_id: u64, deadline_ms: u32, stage: i32) -> DecodedFrame {
        let bytes = encode_message(
            WireKind::LocalConnectionProgress,
            request_id,
            deadline_ms,
            &v2::LocalConnectionProgress { stage },
        )
        .expect("encode progress fixture");
        FrameDecoder::new()
            .feed(&bytes)
            .expect("decode progress fixture")
            .remove(0)
    }

    #[test]
    fn local_progress_rejects_unsolicited_wrong_correlation_unknown_and_excess_frames() {
        let (observer, history) = ProgressObserver::channel(|_| {});
        let mut decoder = LocalProgressDecoder::new(7, observer.clone());
        let stage = v2::LocalConnectionStage::LookingUpAddress as i32;
        for frame in [
            progress_frame(8, 0, stage),
            progress_frame(7, 1, stage),
            progress_frame(7, 0, 0),
            progress_frame(7, 0, 999),
        ] {
            assert!(decoder.consume(&frame).is_err());
        }
        assert_eq!(history.borrow().after(0).count(), 0);
        let frame = progress_frame(7, 0, stage);
        assert!(
            LocalProgressDecoder::new(7, ProgressObserver::default())
                .consume(&frame)
                .is_err()
        );
        let mut decoder = LocalProgressDecoder::new(7, observer);
        for _ in 0..CONNECTION_PROGRESS_CAPACITY {
            assert!(decoder.consume(&frame).expect("valid progress"));
        }
        assert!(decoder.consume(&frame).is_err());
        assert_eq!(history.borrow().after(0).count(), 1);
    }
}
