//! Android's application-scoped native runtime and typed Kotlin boundary.

use std::fmt;
use std::sync::Arc;

use tokio::runtime::Runtime;
use tokio::sync::{OnceCell, watch};

pub mod dns;
pub mod network;
pub mod terminal;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

uniffi::setup_scaffolding!();

/// Errors that can cross the Kotlin boundary without exposing private content.
#[derive(Debug, uniffi::Error)]
pub enum NativeError {
    /// This operation was explicitly cancelled.
    Cancelled,
    /// The application runtime has been closed.
    Closed,
    /// The native executor could not complete the operation.
    RuntimeUnavailable,
    /// Stable domain category; remote diagnostics and secrets are never exposed.
    RequestFailed {
        /// Stable protocol error code for localized Android presentation.
        code: String,
    },
}

impl fmt::Display for NativeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Cancelled => "operation cancelled",
            Self::Closed => "runtime closed",
            Self::RuntimeUnavailable => "native runtime unavailable",
            Self::RequestFailed { code } => code,
        })
    }
}

impl std::error::Error for NativeError {}

/// Content-free application runtime state, delivered through a latest-value slot.
#[derive(Clone, Debug, uniffi::Record)]
pub struct RuntimeState {
    /// Monotonic native state generation.
    pub generation: u64,
    /// Product version from the workspace's single version owner.
    pub version: String,
}

/// One cancellable observation, independent of the application connection owner.
#[derive(Debug, uniffi::Object)]
pub struct NativeOperation {
    cancel: CancellationToken,
}

#[uniffi::export]
impl NativeOperation {
    /// Cancels only this operation. It never closes the application runtime.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// One runtime retained by Android's Application, independent of Activity state.
#[derive(uniffi::Object)]
pub struct NativeRuntime {
    executor: Option<Runtime>,
    state: watch::Sender<RuntimeState>,
    closed: CancellationToken,
    network: Arc<OnceCell<network::Network>>,
    sessions: Arc<zterm_client::unary::SessionUnaryClient>,
}

#[uniffi::export]
impl NativeRuntime {
    /// Creates the application's native executor and latest-state owner.
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>, NativeError> {
        let executor = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("zterm-android")
            .enable_all()
            .build()
            .map_err(|_| NativeError::RuntimeUnavailable)?;
        let (state, _) = watch::channel(RuntimeState {
            generation: 1,
            version: env!("CARGO_PKG_VERSION").to_owned(),
        });
        Ok(Arc::new(Self {
            executor: Some(executor),
            state,
            closed: CancellationToken::new(),
            network: Arc::new(OnceCell::new()),
            sessions: Arc::new(zterm_client::unary::SessionUnaryClient::default()),
        }))
    }

    /// Returns the latest complete native state without starting a subscription.
    pub fn current_state(&self) -> Result<RuntimeState, NativeError> {
        if self.closed.is_cancelled() {
            return Err(NativeError::Closed);
        }
        Ok(self.state.borrow().clone())
    }

    /// Allocates a cancellation handle for one UI operation.
    pub fn new_operation(&self) -> Result<Arc<NativeOperation>, NativeError> {
        self.current_state()?;
        Ok(Arc::new(NativeOperation {
            cancel: CancellationToken::new(),
        }))
    }

    /// Waits for a state newer than the caller's generation on the owned executor.
    /// Dropping the foreign future aborts its waiter, without ending the runtime.
    pub async fn wait_for_state(
        &self,
        after_generation: u64,
        operation: Arc<NativeOperation>,
    ) -> Result<RuntimeState, NativeError> {
        let mut state = self.state.subscribe();
        let closed = self.closed.clone();
        let executor = self.executor.as_ref().ok_or(NativeError::Closed)?;
        let waiter = AbortOnDropHandle::new(executor.spawn(async move {
            loop {
                if operation.cancel.is_cancelled() {
                    return Err(NativeError::Cancelled);
                }
                if closed.is_cancelled() {
                    return Err(NativeError::Closed);
                }
                let latest = state.borrow_and_update().clone();
                if latest.generation > after_generation {
                    return Ok(latest);
                }
                tokio::select! {
                    biased;
                    () = operation.cancel.cancelled() => return Err(NativeError::Cancelled),
                    () = closed.cancelled() => return Err(NativeError::Closed),
                    changed = state.changed() => {
                        changed.map_err(|_| NativeError::Closed)?;
                    }
                }
            }
        }));
        waiter.await.map_err(|_| NativeError::RuntimeUnavailable)?
    }

    /// Explicitly closes networking on the live executor before foreign disposal.
    /// UI/background lifecycle must not call it. Repeated shutdown is idempotent.
    pub async fn shutdown(&self) -> Result<(), NativeError> {
        self.closed.cancel();
        let network = Arc::clone(&self.network);
        let executor = self.executor.as_ref().ok_or(NativeError::Closed)?;
        // Shutdown must run after `closed` cancels ordinary operations. It cannot
        // use on_executor, whose admission/wait is deliberately fenced by closed.
        executor
            .spawn(async move {
                if let Some(network) = network.get() {
                    network.controller.shutdown().await;
                }
            })
            .await
            .map_err(|_| NativeError::RuntimeUnavailable)
    }
}

impl Drop for NativeRuntime {
    fn drop(&mut self) {
        self.closed.cancel();
        if let Some(executor) = self.executor.take() {
            // Foreign future disposal may occur while a Tokio executor is entered.
            executor.shutdown_background();
        }
    }
}

impl NativeRuntime {
    async fn on_executor<T: Send + 'static>(
        &self,
        future: impl std::future::Future<Output = Result<T, NativeError>> + Send + 'static,
    ) -> Result<T, NativeError> {
        if self.closed.is_cancelled() {
            return Err(NativeError::Closed);
        }
        let executor = self.executor.as_ref().ok_or(NativeError::Closed)?;
        let closed = self.closed.clone();
        let task = AbortOnDropHandle::new(executor.spawn(async move {
            tokio::select! { biased; _ = closed.cancelled() => Err(NativeError::Closed), result = future => result }
        }));
        task.await.map_err(|_| NativeError::RuntimeUnavailable)?
    }
}
impl From<zterm_client::error::ClientError> for NativeError {
    fn from(error: zterm_client::error::ClientError) -> Self {
        Self::RequestFailed {
            code: error.kind().code().to_owned(),
        }
    }
}
