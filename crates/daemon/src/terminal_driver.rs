//! Retained PTY drain and latest-state attachment boundary.

use std::collections::VecDeque;
use std::fmt;
use std::io::Read;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use zterm_core::terminal::{
    TerminalCheckpoint, TerminalDeltaResult, TerminalError, TerminalModel, TerminalSize,
    TerminalSnapshot,
};
use zterm_platform::pty::{PtyChildState, PtyError, PtyExitStatus, PtySession, PtySize};

/// Fixed resource limits for one retained terminal driver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalDriverConfig {
    /// Maximum number of PTY byte chunks waiting for the model owner.
    pub byte_channel_capacity: usize,
    /// Maximum number of bytes read from the PTY in one operation.
    pub read_chunk_bytes: usize,
}

impl Default for TerminalDriverConfig {
    fn default() -> Self {
        Self {
            byte_channel_capacity: 8,
            read_chunk_bytes: 8 * 1024,
        }
    }
}

/// Observable bounded-queue and attachment state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalDriverStats {
    /// Configured byte-channel capacity.
    pub byte_channel_capacity: usize,
    /// Chunks currently waiting for the model owner.
    pub pending_chunks: usize,
    /// Largest observed number of pending chunks.
    pub maximum_pending_chunks: usize,
    /// Chunks processed by the model owner.
    pub processed_chunks: u64,
    /// Bytes processed by the model owner.
    pub processed_bytes: u64,
    /// Currently retained terminal attachments.
    pub active_attachments: usize,
}

/// Failure at the daemon terminal-driver boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalDriverError {
    /// A fixed resource limit was zero or could not be represented.
    InvalidConfig(&'static str),
    /// The platform PTY boundary rejected or failed an operation.
    Pty(PtyError),
    /// The host-authoritative terminal model rejected an update.
    Terminal(TerminalError),
    /// The blocking PTY reader failed.
    Read(String),
    /// An internal synchronization primitive was poisoned.
    Synchronization(&'static str),
    /// A bounded wait elapsed.
    Deadline(&'static str),
}

impl fmt::Display for TerminalDriverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(detail) => write!(formatter, "invalid terminal driver: {detail}"),
            Self::Pty(error) => write!(formatter, "terminal driver PTY error: {error}"),
            Self::Terminal(error) => write!(formatter, "terminal model error: {error}"),
            Self::Read(detail) => write!(formatter, "terminal PTY read failed: {detail}"),
            Self::Synchronization(detail) => {
                write!(
                    formatter,
                    "terminal driver synchronization failed: {detail}"
                )
            }
            Self::Deadline(detail) => write!(formatter, "terminal driver deadline: {detail}"),
        }
    }
}

impl std::error::Error for TerminalDriverError {}

impl From<PtyError> for TerminalDriverError {
    fn from(error: PtyError) -> Self {
        Self::Pty(error)
    }
}

impl From<TerminalError> for TerminalDriverError {
    fn from(error: TerminalError) -> Self {
        Self::Terminal(error)
    }
}

/// Host-owned runtime which drains one PTY independently of attachments.
pub struct TerminalDriver {
    session: Arc<Mutex<PtySession>>,
    shared: Arc<SharedTerminal>,
    queue: Arc<ByteQueue>,
    _reader_thread: JoinHandle<()>,
    _model_thread: JoinHandle<()>,
}

impl TerminalDriver {
    /// Starts the blocking reader and the single ordered terminal-model owner.
    pub fn start(
        mut session: PtySession,
        model: TerminalModel,
        config: TerminalDriverConfig,
    ) -> Result<Self, TerminalDriverError> {
        validate_config(config)?;

        let mut reader = session.take_reader()?;
        let session = Arc::new(Mutex::new(session));
        let shared = Arc::new(SharedTerminal::new(model));
        let queue = Arc::new(ByteQueue::new(config.byte_channel_capacity));

        let model_queue = Arc::clone(&queue);
        let model_shared = Arc::clone(&shared);
        let model_session = Arc::clone(&session);
        let model_thread = thread::Builder::new()
            .name("zterm-terminal-model".into())
            .spawn(move || {
                while let Some(bytes) = model_queue.pop() {
                    let update = match model_shared.ingest(&bytes) {
                        Ok(update) => update,
                        Err(error) => {
                            model_queue.complete();
                            model_shared.fail(error);
                            model_queue.abort();
                            return;
                        }
                    };
                    if !update.replies.is_empty() {
                        let result = lock(&model_session, "PTY session").and_then(|mut session| {
                            session.write_input(&update.replies).map_err(Into::into)
                        });
                        if let Err(error) = result {
                            model_queue.complete();
                            model_shared.fail(error);
                            model_queue.abort();
                            return;
                        }
                    }
                    model_shared.record_processed(bytes.len(), update.revision);
                    model_queue.complete();
                }
                model_shared.mark_drain_finished();
            })
            .map_err(|error| TerminalDriverError::Read(error.to_string()))?;

        let reader_queue = Arc::clone(&queue);
        let reader_shared = Arc::clone(&shared);
        let reader_thread = match thread::Builder::new()
            .name("zterm-pty-reader".into())
            .spawn(move || {
                let mut buffer = vec![0_u8; config.read_chunk_bytes];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            if !reader_queue.push(buffer[..count].to_vec()) {
                                break;
                            }
                        }
                        Err(error) => {
                            reader_shared.fail(TerminalDriverError::Read(error.to_string()));
                            break;
                        }
                    }
                }
                reader_queue.finish();
            }) {
            Ok(thread) => thread,
            Err(error) => {
                queue.finish();
                let _ = model_thread.join();
                return Err(TerminalDriverError::Read(error.to_string()));
            }
        };

        Ok(Self {
            session,
            shared,
            queue,
            _reader_thread: reader_thread,
            _model_thread: model_thread,
        })
    }

    /// Creates a latest-state attachment without transferring PTY ownership.
    #[must_use]
    pub fn attach(&self) -> TerminalAttachment {
        self.shared.attachments.fetch_add(1, Ordering::AcqRel);
        TerminalAttachment {
            shared: Arc::clone(&self.shared),
            checkpoint: None,
        }
    }

    /// Writes user input to the hosted PTY.
    pub fn write_input(&self, bytes: &[u8]) -> Result<(), TerminalDriverError> {
        lock(&self.session, "PTY session")?.write_input(bytes)?;
        Ok(())
    }

    /// Resizes both the native PTY and the authoritative terminal model.
    pub fn resize(&self, size: TerminalSize) -> Result<u64, TerminalDriverError> {
        lock(&self.session, "PTY session")?.resize(PtySize::new(size.rows, size.columns))?;
        let revision = self.shared.resize(size)?;
        Ok(revision)
    }

    /// Observes root-child exit without terminating it.
    pub fn try_wait(&self) -> Result<PtyChildState, TerminalDriverError> {
        Ok(lock(&self.session, "PTY session")?.try_wait()?)
    }

    /// Waits for natural root-child exit without invoking the child killer.
    pub fn wait(&self) -> Result<PtyExitStatus, TerminalDriverError> {
        loop {
            let child_state = {
                let mut session = lock(&self.session, "PTY session")?;
                session.try_wait()?
            };
            match child_state {
                PtyChildState::Running => thread::sleep(Duration::from_millis(5)),
                PtyChildState::Exited(status) => return Ok(status),
            }
        }
    }

    /// Explicitly terminates the root child and waits for it.
    pub fn close_explicitly(&self) -> Result<PtyExitStatus, TerminalDriverError> {
        Ok(lock(&self.session, "PTY session")?.close_explicitly()?)
    }

    /// Waits until all bytes read so far have reached the terminal model.
    pub fn wait_until_idle(&self, timeout: Duration) -> Result<(), TerminalDriverError> {
        let deadline = Instant::now() + timeout;
        loop {
            self.shared.check_failure()?;
            if self.queue.is_idle() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(TerminalDriverError::Deadline(
                    "byte channel did not become idle",
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Returns the latest published model revision.
    #[must_use]
    pub fn latest_revision(&self) -> u64 {
        self.shared.revision.load(Ordering::Acquire)
    }

    /// Returns bounded queue and subscriber statistics.
    pub fn stats(&self) -> Result<TerminalDriverStats, TerminalDriverError> {
        let queue = self.queue.stats()?;
        Ok(TerminalDriverStats {
            byte_channel_capacity: self.queue.capacity,
            pending_chunks: queue.pending,
            maximum_pending_chunks: queue.maximum_pending,
            processed_chunks: self.shared.processed_chunks.load(Ordering::Acquire),
            processed_bytes: self.shared.processed_bytes.load(Ordering::Acquire),
            active_attachments: self.shared.attachments.load(Ordering::Acquire),
        })
    }
}

/// Latest-only terminal view owned by one UI or transport attachment.
pub struct TerminalAttachment {
    shared: Arc<SharedTerminal>,
    checkpoint: Option<TerminalCheckpoint>,
}

impl TerminalAttachment {
    /// Waits until the authoritative model advances beyond `revision`.
    pub fn wait_for_revision_after(
        &self,
        revision: u64,
        timeout: Duration,
    ) -> Result<u64, TerminalDriverError> {
        self.shared.wait_for_revision_after(revision, timeout)
    }

    /// Returns one latest merged delta or full resynchronization.
    ///
    /// No intermediate revision queue is retained. The attachment's checkpoint
    /// is replaced by a checkpoint at the exact state returned here.
    pub fn sync_latest(&mut self) -> Result<TerminalDeltaResult, TerminalDriverError> {
        self.shared.check_failure()?;
        let model = lock(&self.shared.model, "terminal model")?;
        let result = self.checkpoint.as_ref().map_or_else(
            || TerminalDeltaResult::Resync(model.snapshot()),
            |checkpoint| model.delta_or_resync(checkpoint),
        );
        self.checkpoint = Some(model.checkpoint());
        Ok(result)
    }

    /// Discards this attachment's stale watermark so the next sync is full.
    pub fn discard_checkpoint(&mut self) {
        self.checkpoint = None;
    }

    /// Returns a full snapshot without changing this attachment's checkpoint.
    pub fn latest_snapshot(&self) -> Result<TerminalSnapshot, TerminalDriverError> {
        self.shared.check_failure()?;
        Ok(lock(&self.shared.model, "terminal model")?.snapshot())
    }
}

impl Drop for TerminalAttachment {
    fn drop(&mut self) {
        self.shared.attachments.fetch_sub(1, Ordering::AcqRel);
    }
}

struct SharedTerminal {
    model: Mutex<TerminalModel>,
    revision: AtomicU64,
    revision_wait: Mutex<RevisionState>,
    revision_changed: Condvar,
    processed_chunks: AtomicU64,
    processed_bytes: AtomicU64,
    attachments: AtomicUsize,
    drain_finished: Mutex<bool>,
}

struct RevisionState {
    latest: u64,
    failure: Option<TerminalDriverError>,
}

impl SharedTerminal {
    fn new(model: TerminalModel) -> Self {
        let revision = model.revision();
        Self {
            model: Mutex::new(model),
            revision: AtomicU64::new(revision),
            revision_wait: Mutex::new(RevisionState {
                latest: revision,
                failure: None,
            }),
            revision_changed: Condvar::new(),
            processed_chunks: AtomicU64::new(0),
            processed_bytes: AtomicU64::new(0),
            attachments: AtomicUsize::new(0),
            drain_finished: Mutex::new(false),
        }
    }

    fn ingest(
        &self,
        bytes: &[u8],
    ) -> Result<zterm_core::terminal::TerminalUpdate, TerminalDriverError> {
        Ok(lock(&self.model, "terminal model")?.ingest(bytes)?)
    }

    fn resize(&self, size: TerminalSize) -> Result<u64, TerminalDriverError> {
        let update = lock(&self.model, "terminal model")?.resize(size)?;
        self.publish_revision(update.revision)?;
        Ok(update.revision)
    }

    fn record_processed(&self, byte_count: usize, revision: u64) {
        self.processed_chunks.fetch_add(1, Ordering::AcqRel);
        self.processed_bytes.fetch_add(
            u64::try_from(byte_count).unwrap_or(u64::MAX),
            Ordering::AcqRel,
        );
        if let Err(error) = self.publish_revision(revision) {
            self.fail(error);
        }
    }

    fn publish_revision(&self, revision: u64) -> Result<(), TerminalDriverError> {
        self.revision.store(revision, Ordering::Release);
        lock(&self.revision_wait, "revision waterline")?.latest = revision;
        self.revision_changed.notify_all();
        Ok(())
    }

    fn wait_for_revision_after(
        &self,
        revision: u64,
        timeout: Duration,
    ) -> Result<u64, TerminalDriverError> {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.revision_wait, "revision waterline")?;
        loop {
            if let Some(error) = state.failure.clone() {
                return Err(error);
            }
            if state.latest > revision {
                return Ok(state.latest);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(TerminalDriverError::Deadline(
                    "terminal revision did not advance",
                ));
            }
            let (guard, wait) = self
                .revision_changed
                .wait_timeout(state, deadline.saturating_duration_since(now))
                .map_err(|_| TerminalDriverError::Synchronization("revision waterline"))?;
            state = guard;
            if wait.timed_out() && state.latest <= revision {
                if let Some(error) = state.failure.clone() {
                    return Err(error);
                }
                return Err(TerminalDriverError::Deadline(
                    "terminal revision did not advance",
                ));
            }
        }
    }

    fn fail(&self, error: TerminalDriverError) {
        if let Ok(mut state) = self.revision_wait.lock()
            && state.failure.is_none()
        {
            state.failure = Some(error);
        }
        self.revision_changed.notify_all();
    }

    fn check_failure(&self) -> Result<(), TerminalDriverError> {
        if let Some(error) = lock(&self.revision_wait, "revision waterline")?
            .failure
            .clone()
        {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn mark_drain_finished(&self) {
        if let Ok(mut finished) = self.drain_finished.lock() {
            *finished = true;
        }
        self.revision_changed.notify_all();
    }
}

struct ByteQueue {
    capacity: usize,
    state: Mutex<ByteQueueState>,
    not_empty: Condvar,
    not_full: Condvar,
}

struct ByteQueueState {
    chunks: VecDeque<Vec<u8>>,
    maximum_pending: usize,
    in_flight: usize,
    finished: bool,
    aborted: bool,
}

struct ByteQueueStats {
    pending: usize,
    maximum_pending: usize,
}

impl ByteQueue {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            state: Mutex::new(ByteQueueState {
                chunks: VecDeque::with_capacity(capacity),
                maximum_pending: 0,
                in_flight: 0,
                finished: false,
                aborted: false,
            }),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
        }
    }

    fn push(&self, bytes: Vec<u8>) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        while state.chunks.len() == self.capacity && !state.aborted {
            let Ok(next) = self.not_full.wait(state) else {
                return false;
            };
            state = next;
        }
        if state.aborted {
            return false;
        }
        state.chunks.push_back(bytes);
        state.maximum_pending = state.maximum_pending.max(state.chunks.len());
        self.not_empty.notify_one();
        true
    }

    fn pop(&self) -> Option<Vec<u8>> {
        let Ok(mut state) = self.state.lock() else {
            return None;
        };
        while state.chunks.is_empty() && !state.finished && !state.aborted {
            let Ok(next) = self.not_empty.wait(state) else {
                return None;
            };
            state = next;
        }
        let bytes = state.chunks.pop_front();
        if bytes.is_some() {
            state.in_flight = state.in_flight.saturating_add(1);
            self.not_full.notify_one();
        }
        bytes
    }

    fn complete(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.in_flight = state.in_flight.saturating_sub(1);
        }
        self.not_full.notify_all();
    }

    fn finish(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.finished = true;
        }
        self.not_empty.notify_all();
    }

    fn abort(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.aborted = true;
        }
        self.not_empty.notify_all();
        self.not_full.notify_all();
    }

    fn is_idle(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.chunks.is_empty() && state.in_flight == 0)
    }

    fn stats(&self) -> Result<ByteQueueStats, TerminalDriverError> {
        let state = lock(&self.state, "PTY byte channel")?;
        Ok(ByteQueueStats {
            pending: state.chunks.len(),
            maximum_pending: state.maximum_pending,
        })
    }
}

fn lock<'a, T>(
    mutex: &'a Mutex<T>,
    name: &'static str,
) -> Result<MutexGuard<'a, T>, TerminalDriverError> {
    mutex
        .lock()
        .map_err(|_| TerminalDriverError::Synchronization(name))
}

fn validate_config(config: TerminalDriverConfig) -> Result<(), TerminalDriverError> {
    if config.byte_channel_capacity == 0 {
        return Err(TerminalDriverError::InvalidConfig(
            "byte channel capacity must be non-zero",
        ));
    }
    if config.read_chunk_bytes == 0 {
        return Err(TerminalDriverError::InvalidConfig(
            "read chunk size must be non-zero",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_queue_limits_are_rejected() {
        assert_eq!(
            validate_config(TerminalDriverConfig {
                byte_channel_capacity: 0,
                read_chunk_bytes: 1,
            }),
            Err(TerminalDriverError::InvalidConfig(
                "byte channel capacity must be non-zero"
            ))
        );
        assert_eq!(
            validate_config(TerminalDriverConfig {
                byte_channel_capacity: 1,
                read_chunk_bytes: 0,
            }),
            Err(TerminalDriverError::InvalidConfig(
                "read chunk size must be non-zero"
            ))
        );
    }

    #[test]
    fn bounded_queue_never_exceeds_its_capacity() {
        let queue = Arc::new(ByteQueue::new(2));
        assert!(queue.push(vec![1]));
        assert!(queue.push(vec![2]));
        assert_eq!(queue.pop(), Some(vec![1]));
        assert!(queue.push(vec![3]));
        let stats = queue.stats().expect("queue stats");
        assert_eq!(stats.maximum_pending, 2);
        assert!(stats.pending <= 2);
    }
}
