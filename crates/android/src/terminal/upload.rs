//! Retained upload operation. The terminal actor alone owns pause and final input.

use super::{NativeError, failure};
use std::sync::{Arc, Mutex};
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;
use zterm_client::upload::{UploadConnector, UploadOrigin};
use zterm_core::upload::{MAX_UPLOAD_BYTES, UploadMetadata, UploadPhase, UploadedFile};

/// Inclusive source byte limit consumed by Android's ContentResolver staging.
#[uniffi::export]
pub fn upload_limit_bytes() -> u64 {
    MAX_UPLOAD_BYTES
}

/// Metadata-only progress, independent of semantic frame generations.
#[derive(Clone, uniffi::Record)]
pub struct NativeUploadState {
    /// Monotonic latest-value generation.
    pub generation: u64,
    /// preparing/uploading/finishing/inserting/completed/cancelled/failed.
    pub phase: String,
    /// Exact host-accepted bytes, never optimistic local reads.
    pub accepted_bytes: u64,
    /// Total bytes, absent until the source is staged.
    pub total_bytes: Option<u64>,
    /// Content-free error code for localized display.
    pub error: Option<String>,
}

struct PreparedUpload {
    path: String,
    size: u64,
    extension: String,
}

/// One explicit upload intent retained by the Repository across Activity recreation.
#[derive(uniffi::Object)]
pub struct NativeUpload {
    state: watch::Sender<NativeUploadState>,
    start: Mutex<Option<oneshot::Sender<PreparedUpload>>>,
    cancel: CancellationToken,
}

#[uniffi::export]
impl NativeUpload {
    /// Submits one private cache file; bytes never cross UniFFI as a whole-file array.
    pub fn start(&self, path: String, size: u64, extension: String) -> Result<(), NativeError> {
        UploadMetadata::new(size, &extension).map_err(|kind| failure(kind.code()))?;
        if self.cancel.is_cancelled() {
            return Err(NativeError::Cancelled);
        }
        let sender = self
            .start
            .lock()
            .map_err(|_| NativeError::RuntimeUnavailable)?
            .take()
            .ok_or_else(|| failure("upload_already_started"))?;
        sender
            .send(PreparedUpload {
                path,
                size,
                extension,
            })
            .map_err(|_| NativeError::Closed)
    }
    /// Cancels source preparation or transfer. Completed remote files are retained.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    /// Current metadata without copying terminal rows.
    pub fn current_state(&self) -> NativeUploadState {
        self.state.borrow().clone()
    }
    /// Waits for one newer generation; cancelling a UI waiter does not cancel upload.
    pub async fn wait_for_progress(
        &self,
        after_generation: u64,
    ) -> Result<NativeUploadState, NativeError> {
        let mut receiver = self.state.subscribe();
        loop {
            {
                let state = receiver.borrow_and_update();
                if state.generation > after_generation {
                    return Ok(state.clone());
                }
            }
            receiver.changed().await.map_err(|_| NativeError::Closed)?;
        }
    }
}
impl NativeUpload {
    pub(super) fn cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
    fn update(&self, phase: &str, accepted_bytes: u64, total_bytes: Option<u64>) {
        self.state.send_modify(|state| {
            state.generation += 1;
            state.phase = phase.into();
            state.accepted_bytes = accepted_bytes;
            state.total_bytes = total_bytes;
        });
    }
    pub(super) fn inserting(&self) {
        let previous = self.current_state();
        self.update("inserting", previous.accepted_bytes, previous.total_bytes);
    }
    pub(super) fn finish(&self, result: Result<(), NativeError>) {
        self.state.send_modify(|state| {
            state.generation += 1;
            match result {
                Ok(()) => {
                    state.phase = "completed".into();
                    state.error = None;
                }
                Err(error) => {
                    let code = match error {
                        NativeError::RequestFailed { code } => code,
                        NativeError::Cancelled | NativeError::Closed => "cancelled".into(),
                        _ => "runtime_unavailable".into(),
                    };
                    state.phase = if code == "cancelled" {
                        "cancelled"
                    } else {
                        "failed"
                    }
                    .into();
                    state.error = Some(code);
                }
            }
        });
    }
}

pub(super) struct ActiveUpload {
    pub(super) handle: Arc<NativeUpload>,
    pub(super) origin: UploadOrigin,
    pub(super) input_epoch: u64,
    task: tokio::task::JoinHandle<Result<UploadedFile, NativeError>>,
}
impl ActiveUpload {
    pub(super) fn new(
        connector: Arc<dyn UploadConnector>,
        origin: UploadOrigin,
        input_epoch: u64,
        parent: &CancellationToken,
    ) -> Self {
        let (start, receiver) = oneshot::channel();
        let (state, _) = watch::channel(NativeUploadState {
            generation: 1,
            phase: "preparing".into(),
            accepted_bytes: 0,
            total_bytes: None,
            error: None,
        });
        let handle = Arc::new(NativeUpload {
            state,
            start: Mutex::new(Some(start)),
            cancel: parent.child_token(),
        });
        let operation = Arc::clone(&handle);
        let task = tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = operation.cancel.cancelled() => Err(NativeError::Cancelled),
                result = transfer(connector, origin, &operation, receiver) => result,
            }
        });
        Self {
            handle,
            origin,
            input_epoch,
            task,
        }
    }
}
impl Drop for ActiveUpload {
    fn drop(&mut self) {
        self.handle.cancel();
        self.task.abort();
    }
}
pub(super) async fn wait_result(
    operation: &mut Option<ActiveUpload>,
) -> Result<UploadedFile, NativeError> {
    let Some(operation) = operation else {
        return std::future::pending().await;
    };
    (&mut operation.task)
        .await
        .map_err(|_| NativeError::Cancelled)?
}

async fn transfer(
    connector: Arc<dyn UploadConnector>,
    origin: UploadOrigin,
    operation: &NativeUpload,
    source: oneshot::Receiver<PreparedUpload>,
) -> Result<UploadedFile, NativeError> {
    let source = source.await.map_err(|_| NativeError::Cancelled)?;
    let metadata =
        UploadMetadata::new(source.size, &source.extension).map_err(|kind| failure(kind.code()))?;
    let file = tokio::fs::File::open(source.path)
        .await
        .map_err(|_| failure("upload_source_invalid"))?;
    let info = file
        .metadata()
        .await
        .map_err(|_| failure("upload_source_invalid"))?;
    if !info.is_file() || info.len() != metadata.size() {
        return Err(failure("upload_source_invalid"));
    }
    operation.update("uploading", 0, Some(metadata.size()));
    let (progress, mut observed) = watch::channel(zterm_client::upload::preparing());
    let upload = zterm_client::upload::upload(
        &*connector,
        origin.target,
        origin.binding,
        metadata,
        file,
        &progress,
        &operation.cancel,
    );
    tokio::pin!(upload);
    let mut tick = tokio::time::interval(zterm_core::upload::UPLOAD_PROGRESS_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            result = &mut upload => {
                if let Ok(file) = &result { operation.update("finishing", file.size(), Some(file.size())); }
                return result.map_err(Into::into);
            }
            _ = tick.tick() => {
                if observed.has_changed().unwrap_or(false) {
                    let next = *observed.borrow_and_update();
                    let phase = match next.phase { UploadPhase::Preparing => "preparing", UploadPhase::Uploading => "uploading", UploadPhase::Finishing | UploadPhase::Completed => "finishing" };
                    operation.update(phase, next.accepted_bytes, next.total_bytes);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zterm_client::{
        error::ClientError, model::ResolvedSessionTarget, transport::TransportFuture,
        upload::UploadConnection,
    };
    use zterm_core::{AttachmentId, SessionId, upload::UploadBinding};

    struct NoNetwork;
    impl UploadConnector for NoNetwork {
        fn open_upload(
            &self,
            _: ResolvedSessionTarget,
        ) -> TransportFuture<'_, Result<UploadConnection, ClientError>> {
            panic!("preparation cancellation must not open a stream")
        }
    }
    fn operation(cancellation: &CancellationToken) -> ActiveUpload {
        ActiveUpload::new(
            Arc::new(NoNetwork),
            UploadOrigin {
                target: ResolvedSessionTarget::local(),
                binding: UploadBinding {
                    session_id: SessionId::from_array([1; 16]),
                    attachment_id: AttachmentId::from_array([2; 16]),
                },
            },
            1,
            cancellation,
        )
    }
    #[tokio::test]
    async fn abandoned_reservation_cancels_without_network_and_late_observer_gets_result() {
        let cancellation = CancellationToken::new();
        let operation = operation(&cancellation);
        let handle = Arc::clone(&operation.handle);
        let mut active = Some(operation);
        cancellation.cancel();
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(1), wait_result(&mut active))
                .await
                .expect("bounded cancellation");
        assert!(matches!(result, Err(NativeError::Cancelled)));
        handle.finish(result.map(|_| ()));
        let state = handle
            .wait_for_progress(0)
            .await
            .expect("latest completion");
        assert_eq!(state.phase, "cancelled");
        assert_eq!(state.accepted_bytes, 0);
        assert!(matches!(
            handle.start("unused".into(), 0, String::new()),
            Err(NativeError::Cancelled)
        ));
    }
    #[tokio::test]
    async fn oversize_does_not_consume_source_slot_and_repeated_start_never_replaces_it() {
        let operation = operation(&CancellationToken::new());
        assert!(
            matches!(operation.handle.start("unused".into(), MAX_UPLOAD_BYTES + 1, String::new()), Err(NativeError::RequestFailed { code }) if code == "upload_too_large")
        );
        operation
            .handle
            .start("unused".into(), 0, String::new())
            .expect("first prepared source");
        assert!(
            matches!(operation.handle.start("replacement".into(), 0, String::new()), Err(NativeError::RequestFailed { code }) if code == "upload_already_started")
        );
        operation.handle.cancel();
    }
}
