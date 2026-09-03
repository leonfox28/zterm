//! Retained PTY drain and latest-state attachment boundary.

use std::collections::VecDeque;
use std::fmt;
use std::io::Read;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tokio::sync::watch;
use zterm_core::Revision;
use zterm_core::terminal::{
    TerminalDeltaResult, TerminalHistoryCursor, TerminalHistoryDirection, TerminalHistoryResult,
    TerminalHistoryWindowQuery, TerminalHistoryWindowResult, TerminalScrollAction,
    TerminalScrollMetrics, TerminalSize, TerminalSnapshot, TerminalViewportResult,
};
use zterm_platform::pty::{
    PtyChild, PtyChildInterrupt, PtyChildState, PtyError, PtyExitStatus, PtyIo, PtySession, PtySize,
};
use zterm_terminal::{TerminalCheckpoint, TerminalError, TerminalModel};

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
    io: Arc<Mutex<PtyIo>>,
    child: Arc<Mutex<PtyChild>>,
    interrupt: PtyChildInterrupt,
    ownership: TerminalDriverOwnership,
    shared: Arc<SharedTerminal>,
    queue: Arc<ByteQueue>,
    reader_thread: Option<JoinHandle<()>>,
    model_thread: Option<JoinHandle<()>>,
    finalized: bool,
}

/// Owner-only child interruption capability for one terminal driver.
///
/// It is intentionally independent from the PTY writer mutex so daemon
/// shutdown can terminate a child even while its session actor is blocked in
/// a write or flush. This capability is never exposed through attachments.
#[derive(Clone)]
pub(crate) struct TerminalDriverInterrupt {
    child: Arc<Mutex<PtyChild>>,
    interrupt: PtyChildInterrupt,
}

impl TerminalDriverInterrupt {
    pub(crate) fn close_explicitly(&self) -> Result<PtyExitStatus, TerminalDriverError> {
        Ok(cleanup_lock(&self.child).close_explicitly()?)
    }

    pub(crate) fn interrupt(&self) {
        let _ = self.interrupt.interrupt();
    }
}

/// Completion signal for the child and both terminal-driver threads.
/// Actor ownership is not released until this boundary is terminally true.
#[derive(Clone)]
pub(crate) struct TerminalDriverOwnership {
    inner: Arc<TerminalDriverOwnershipInner>,
}

struct TerminalDriverOwnershipInner {
    released: Mutex<bool>,
    changed: Condvar,
}

impl TerminalDriverOwnership {
    fn new() -> Self {
        Self {
            inner: Arc::new(TerminalDriverOwnershipInner {
                released: Mutex::new(false),
                changed: Condvar::new(),
            }),
        }
    }

    fn release(&self) {
        *cleanup_lock(&self.inner.released) = true;
        self.inner.changed.notify_all();
    }

    pub(crate) fn wait_released(&self) {
        let mut released = cleanup_lock(&self.inner.released);
        while !*released {
            released = match self.inner.changed.wait(released) {
                Ok(released) => released,
                Err(poisoned) => {
                    self.inner.released.clear_poison();
                    poisoned.into_inner()
                }
            };
        }
    }

    #[cfg(test)]
    fn is_released(&self) -> bool {
        *cleanup_lock(&self.inner.released)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartFailureInjection {
    ModelThread,
    ReaderThread,
}

impl TerminalDriver {
    /// Starts the blocking reader and the single ordered terminal-model owner.
    pub fn start(
        session: PtySession,
        model: TerminalModel,
        config: TerminalDriverConfig,
    ) -> Result<Self, TerminalDriverError> {
        Self::start_inner(session, model, config, None)
    }

    fn start_inner(
        session: PtySession,
        model: TerminalModel,
        config: TerminalDriverConfig,
        #[cfg(test)] failure: Option<StartFailureInjection>,
        #[cfg(not(test))] _failure: Option<()>,
    ) -> Result<Self, TerminalDriverError> {
        validate_config(config)?;

        let parts = session.into_driver_parts()?;
        let mut reader = parts.reader;
        let io = Arc::new(Mutex::new(parts.io));
        let child = Arc::new(Mutex::new(parts.child));
        let interrupt = parts.interrupt;
        let ownership = TerminalDriverOwnership::new();
        let shared = Arc::new(SharedTerminal::new(model));
        let queue = Arc::new(ByteQueue::new(config.byte_channel_capacity));

        let model_queue = Arc::clone(&queue);
        let model_shared = Arc::clone(&shared);
        let model_io = Arc::clone(&io);
        #[cfg(test)]
        let inject_model_failure = failure == Some(StartFailureInjection::ModelThread);
        #[cfg(not(test))]
        let inject_model_failure = false;
        let model_thread = if inject_model_failure {
            Err(std::io::Error::other(
                "injected terminal model thread start failure",
            ))
        } else {
            thread::Builder::new()
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
                            let result = lock(&model_io, "PTY I/O").and_then(|mut io| {
                                io.write_input(&update.replies).map_err(Into::into)
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
        };
        let model_thread = match model_thread {
            Ok(thread) => thread,
            Err(error) => {
                let cleanup = close_startup_child(&child);
                return Err(cleanup.unwrap_or_else(|| TerminalDriverError::Read(error.to_string())));
            }
        };

        #[cfg(test)]
        let inject_reader_failure = failure == Some(StartFailureInjection::ReaderThread);
        #[cfg(not(test))]
        let inject_reader_failure = false;
        let reader_queue = Arc::clone(&queue);
        let reader_shared = Arc::clone(&shared);
        let reader_thread = if inject_reader_failure {
            Err(std::io::Error::other(
                "injected PTY reader thread start failure",
            ))
        } else {
            thread::Builder::new()
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
                })
        };
        let reader_thread = match reader_thread {
            Ok(thread) => thread,
            Err(error) => {
                queue.abort();
                let cleanup = close_startup_child(&child);
                let join = model_thread
                    .join()
                    .err()
                    .map(|_| TerminalDriverError::Synchronization("terminal model thread"));
                return Err(cleanup
                    .or(join)
                    .unwrap_or_else(|| TerminalDriverError::Read(error.to_string())));
            }
        };

        Ok(Self {
            io,
            child,
            interrupt,
            ownership,
            shared,
            queue,
            reader_thread: Some(reader_thread),
            model_thread: Some(model_thread),
            finalized: false,
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
        lock(&self.io, "PTY I/O")?.write_input(bytes)?;
        Ok(())
    }

    /// Resizes both the native PTY and the authoritative terminal model.
    pub fn resize(&self, size: TerminalSize) -> Result<Revision, TerminalDriverError> {
        let mut model = lock(&self.shared.model, "terminal model")?;
        model.preflight_resize(size)?;
        lock(&self.io, "PTY I/O")?.resize(PtySize::new(size.rows, size.columns))?;
        let revision = model.resize(size)?.revision;
        drop(model);
        self.shared.publish_revision(revision)?;
        Ok(revision)
    }

    /// Subscribes to the latest revision without retaining per-revision events.
    #[must_use]
    pub fn revision_watch(&self) -> watch::Receiver<Revision> {
        self.shared.revision_sender.subscribe()
    }

    /// Returns a recorded model/reader failure without mutating the child.
    pub fn check_health(&self) -> Result<(), TerminalDriverError> {
        self.shared.check_failure()
    }

    /// Observes root-child exit without terminating it.
    pub fn try_wait(&self) -> Result<PtyChildState, TerminalDriverError> {
        Ok(lock(&self.child, "PTY child")?.try_wait()?)
    }

    /// Waits for natural root-child exit without invoking the child killer.
    pub fn wait(&self) -> Result<PtyExitStatus, TerminalDriverError> {
        loop {
            let child_state = {
                let mut child = lock(&self.child, "PTY child")?;
                child.try_wait()?
            };
            match child_state {
                PtyChildState::Running => thread::sleep(Duration::from_millis(5)),
                PtyChildState::Exited(status) => return Ok(status),
            }
        }
    }

    /// Explicitly terminates the root child and waits for it.
    pub fn close_explicitly(&self) -> Result<PtyExitStatus, TerminalDriverError> {
        Ok(lock(&self.child, "PTY child")?.close_explicitly()?)
    }

    /// Returns the owner-only close capability used by bounded daemon shutdown.
    pub(crate) fn interrupt_handle(&self) -> TerminalDriverInterrupt {
        TerminalDriverInterrupt {
            child: Arc::clone(&self.child),
            interrupt: self.interrupt.clone(),
        }
    }

    pub(crate) fn ownership_handle(&self) -> TerminalDriverOwnership {
        self.ownership.clone()
    }

    /// Consumes a naturally exited driver, drains queued bytes, and joins its threads.
    pub fn finalize_natural(self) -> Result<PtyExitStatus, TerminalDriverError> {
        self.finalize(false)
    }

    /// Explicitly closes a driver, drains queued bytes, and joins its threads.
    pub fn finalize_explicit(self) -> Result<PtyExitStatus, TerminalDriverError> {
        self.finalize(true)
    }

    fn finalize(mut self, explicit: bool) -> Result<PtyExitStatus, TerminalDriverError> {
        let status = if explicit {
            self.close_explicitly()
        } else {
            self.wait()
        };
        let reader = self
            .reader_thread
            .take()
            .expect("live terminal driver owns its reader thread")
            .join()
            .map_err(|_| TerminalDriverError::Synchronization("PTY reader thread"));
        let model = self
            .model_thread
            .take()
            .expect("live terminal driver owns its model thread")
            .join()
            .map_err(|_| TerminalDriverError::Synchronization("terminal model thread"));
        let health = self.shared.check_failure();
        if status.is_ok() {
            // Both joins have returned (including a panic result), and the
            // child status proves it is no longer owned. Model health affects
            // the reported result, not lifecycle truth.
            self.ownership.release();
            self.finalized = true;
        }
        let status = status?;
        reader?;
        model?;
        health?;
        Ok(status)
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
    pub fn latest_revision(&self) -> Revision {
        Revision::new(self.shared.revision.load(Ordering::Acquire))
    }

    /// Returns a full latest snapshot without creating an attachment.
    pub fn latest_snapshot(&self) -> Result<TerminalSnapshot, TerminalDriverError> {
        self.shared.check_failure()?;
        Ok(lock(&self.shared.model, "terminal model")?.snapshot())
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

impl Drop for TerminalDriver {
    fn drop(&mut self) {
        if self.finalized {
            return;
        }
        // Last-resort unwind ownership is transferred without joining on the
        // caller. Fast interruption and queue abort happen first; a dedicated
        // reaper performs the truthful child wait and thread joins. If reaper
        // thread creation itself fails, dropping the closure safely detaches
        // already-interrupted JoinHandles and deliberately leaves the
        // ownership signal unreleased.
        self.queue.abort();
        let _ = self.interrupt.interrupt();
        let child = Arc::clone(&self.child);
        let queue = Arc::clone(&self.queue);
        let ownership = self.ownership.clone();
        let reader = self.reader_thread.take();
        let model = self.model_thread.take();
        spawn_background_reaper("zterm-terminal-reaper", move || {
            let child = cleanup_lock(&child).close_explicitly();
            queue.abort();
            if let Some(reader) = reader {
                let _ = reader.join();
            }
            if let Some(model) = model {
                let _ = model.join();
            }
            if child.is_ok() {
                ownership.release();
            }
        });
        self.finalized = true;
    }
}

/// Latest-only terminal view owned by one UI or transport attachment.
pub struct TerminalAttachment {
    shared: Arc<SharedTerminal>,
    checkpoint: Option<TerminalCheckpoint>,
}

impl TerminalAttachment {
    /// Returns the revision retained by this view's latest checkpoint.
    #[must_use]
    pub(crate) fn checkpoint_revision(&self) -> Option<Revision> {
        self.checkpoint.as_ref().map(TerminalCheckpoint::revision)
    }

    /// Subscribes to the driver's latest-only revision watermark.
    #[must_use]
    pub fn revision_watch(&self) -> watch::Receiver<Revision> {
        self.shared.revision_sender.subscribe()
    }

    /// Waits until the authoritative model advances beyond `revision`.
    pub fn wait_for_revision_after(
        &self,
        revision: Revision,
        timeout: Duration,
    ) -> Result<Revision, TerminalDriverError> {
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

    /// Returns one bounded main-screen history page without advancing this
    /// attachment's latest-state checkpoint.
    pub fn history_page(
        &self,
        direction: TerminalHistoryDirection,
        cursor: Option<TerminalHistoryCursor>,
        maximum_rows: usize,
    ) -> Result<TerminalHistoryResult, TerminalDriverError> {
        self.shared.check_failure()?;
        lock(&self.shared.model, "terminal model")?
            .history_page(direction, cursor, maximum_rows)
            .map_err(Into::into)
    }

    /// Projects one complete semantic viewport without advancing the live checkpoint.
    pub fn scroll_viewport(
        &self,
        previous: Option<TerminalScrollMetrics>,
        action: TerminalScrollAction,
    ) -> Result<TerminalViewportResult, TerminalDriverError> {
        self.shared.check_failure()?;
        Ok(lock(&self.shared.model, "terminal model")?.scroll_viewport(previous, action))
    }

    /// Projects one stateless contiguous history window under one model lock.
    pub fn history_window(
        &self,
        query: TerminalHistoryWindowQuery,
    ) -> Result<TerminalHistoryWindowResult, TerminalDriverError> {
        self.shared.check_failure()?;
        Ok(lock(&self.shared.model, "terminal model")?.history_window(query))
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
    revision_sender: watch::Sender<Revision>,
    processed_chunks: AtomicU64,
    processed_bytes: AtomicU64,
    attachments: AtomicUsize,
    drain_finished: Mutex<bool>,
}

struct RevisionState {
    latest: Revision,
    failure: Option<TerminalDriverError>,
}

impl SharedTerminal {
    fn new(model: TerminalModel) -> Self {
        let revision = model.revision();
        let (revision_sender, _) = watch::channel(revision);
        Self {
            model: Mutex::new(model),
            revision: AtomicU64::new(revision.get()),
            revision_wait: Mutex::new(RevisionState {
                latest: revision,
                failure: None,
            }),
            revision_changed: Condvar::new(),
            revision_sender,
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

    fn record_processed(&self, byte_count: usize, revision: Revision) {
        self.processed_chunks.fetch_add(1, Ordering::AcqRel);
        self.processed_bytes.fetch_add(
            u64::try_from(byte_count).unwrap_or(u64::MAX),
            Ordering::AcqRel,
        );
        if let Err(error) = self.publish_revision(revision) {
            self.fail(error);
        }
    }

    fn publish_revision(&self, revision: Revision) -> Result<(), TerminalDriverError> {
        self.revision.store(revision.get(), Ordering::Release);
        lock(&self.revision_wait, "revision waterline")?.latest = revision;
        self.revision_sender.send_replace(revision);
        self.revision_changed.notify_all();
        Ok(())
    }

    fn wait_for_revision_after(
        &self,
        revision: Revision,
        timeout: Duration,
    ) -> Result<Revision, TerminalDriverError> {
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
        while state.chunks.len() == self.capacity && !state.finished && !state.aborted {
            let Ok(next) = self.not_full.wait(state) else {
                return false;
            };
            state = next;
        }
        if state.finished || state.aborted {
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
        self.not_full.notify_all();
    }

    fn abort(&self) {
        cleanup_lock(&self.state).aborted = true;
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

fn cleanup_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            mutex.clear_poison();
            poisoned.into_inner()
        }
    }
}

pub(crate) fn spawn_background_reaper(name: &'static str, work: impl FnOnce() + Send + 'static) {
    // `spawn` consumes and drops `work` on failure. All callers must therefore
    // interrupt/abort before handing exclusively-taken handles to this helper.
    let _ = thread::Builder::new().name(name.into()).spawn(work);
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

fn close_startup_child(child: &Arc<Mutex<PtyChild>>) -> Option<TerminalDriverError> {
    match lock(child, "PTY child") {
        Ok(mut child) => child.close_explicitly().err().map(Into::into),
        Err(error) => Some(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::path::Path;

    #[cfg(unix)]
    use zterm_platform::pty::{ExplicitPtyCommand, PtyHost};

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

    #[test]
    fn finished_or_aborted_queue_rejects_late_enqueues() {
        let finished = Arc::new(ByteQueue::new(1));
        assert!(finished.push(vec![1]));
        let late = Arc::clone(&finished);
        let (started, observed) = std::sync::mpsc::sync_channel(1);
        let (done, result) = std::sync::mpsc::sync_channel(1);
        let producer = thread::spawn(move || {
            started.send(()).expect("late producer starts");
            let _ = done.send(late.push(vec![2]));
        });
        observed.recv().expect("late producer observed");
        finished.finish();
        assert!(
            !result
                .recv_timeout(Duration::from_secs(1))
                .expect("finish wakes blocked producer"),
            "a finished queue must reject a late chunk"
        );
        producer.join().expect("late producer joins");
        assert_eq!(finished.pop(), Some(vec![1]));
        assert_eq!(finished.pop(), None);

        let aborted = ByteQueue::new(1);
        aborted.abort();
        assert!(!aborted.push(vec![3]));
    }

    #[cfg(unix)]
    #[test]
    fn drop_returns_before_blocked_io_while_reaper_releases_child_and_threads() {
        let shell = Path::new("/bin/sh");
        if !shell.is_file() {
            return;
        }
        let cwd = std::env::current_dir().expect("current test directory");
        let session = PtyHost::new()
            .spawn(
                ExplicitPtyCommand::new(shell, cwd)
                    .arg("-c")
                    .arg("trap '' HUP; printf 'DROP-READY\\r\\n'; while :; do :; done"),
                PtySize::new(24, 80),
            )
            .expect("spawn drop/reaper fixture");
        let process_id = session.process_id().expect("fixture process id");
        let model =
            TerminalModel::new(TerminalSize::new(24, 80), 0).expect("fixture terminal model");
        let driver = TerminalDriver::start(session, model, TerminalDriverConfig::default())
            .expect("fixture driver starts");
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = driver.latest_snapshot().expect("fixture snapshot");
            if snapshot
                .screen_ansi
                .windows(b"DROP-READY".len())
                .any(|window| window == b"DROP-READY")
            {
                break;
            }
            assert!(
                Instant::now() < ready_deadline,
                "fixture never became ready"
            );
            thread::sleep(Duration::from_millis(2));
        }

        // Hold the writer ownership mutex while the PTY reader is blocked in
        // its next read. Drop must need neither one.
        let io = Arc::clone(&driver.io);
        let (writer_locked, observed_lock) = std::sync::mpsc::sync_channel(1);
        let (release_writer, writer_release) = std::sync::mpsc::sync_channel(1);
        let writer = thread::spawn(move || {
            let _io = io.lock().expect("writer mutex");
            writer_locked.send(()).expect("writer lock observer");
            writer_release.recv().expect("writer release");
        });
        observed_lock
            .recv()
            .expect("writer is deliberately blocked");

        let ownership = driver.ownership_handle();
        let (dropped, drop_result) = std::sync::mpsc::sync_channel(1);
        thread::spawn(move || {
            drop(driver);
            let _ = dropped.send(());
        });
        drop_result
            .recv_timeout(Duration::from_millis(100))
            .expect("Drop never waits for PTY child/reader/writer cleanup");
        assert!(
            !ownership.is_released(),
            "HUP-resistant child cannot be reported released before reaping"
        );
        release_writer.send(()).expect("release writer mutex");
        writer.join().expect("writer owner joins");

        let process_id = i32::try_from(process_id).expect("pid fits pid_t");
        let deadline = Instant::now() + Duration::from_secs(3);
        while !ownership.is_released() {
            assert!(
                Instant::now() < deadline,
                "background reaper never completed"
            );
            thread::sleep(Duration::from_millis(5));
        }
        loop {
            let result = nix::sys::signal::kill(nix::unistd::Pid::from_raw(process_id), None);
            if result == Err(nix::errno::Errno::ESRCH) {
                break;
            }
            assert!(Instant::now() < deadline, "reaper left child running");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(unix)]
    #[test]
    fn thread_start_failures_close_spawned_children_and_join_started_owners() {
        let shell = Path::new("/bin/sh");
        if !shell.is_file() {
            return;
        }
        for failure in [
            StartFailureInjection::ModelThread,
            StartFailureInjection::ReaderThread,
        ] {
            let cwd = std::env::current_dir().expect("current test directory");
            let session = PtyHost::new()
                .spawn(
                    ExplicitPtyCommand::new(shell, cwd)
                        .arg("-c")
                        .arg("trap '' HUP; while :; do sleep 1; done"),
                    PtySize::new(24, 80),
                )
                .expect("spawn startup-cleanup fixture");
            let process_id = session.process_id().expect("fixture process id");
            let model =
                TerminalModel::new(TerminalSize::new(24, 80), 0).expect("fixture terminal model");

            let error = TerminalDriver::start_inner(
                session,
                model,
                TerminalDriverConfig::default(),
                Some(failure),
            )
            .err()
            .expect("injected terminal-driver thread start failure");
            assert!(matches!(error, TerminalDriverError::Read(_)));

            let process_id = i32::try_from(process_id).expect("process id fits pid_t");
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let result = nix::sys::signal::kill(nix::unistd::Pid::from_raw(process_id), None);
                if result == Err(nix::errno::Errno::ESRCH) {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "{failure:?} left child {process_id} running"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
