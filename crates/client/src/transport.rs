//! Injected semantic stream adapters and cancellation-safe attachment epochs.
use crate::{error::ClientError, model::ResolvedSessionTarget, protocol::*};
use std::{future::Future, pin::Pin};
use zterm_proto::{DecodedFrame, v2};

/// Sendable, borrowed adapter operation.
pub type TransportFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A Session frame or address-free adapter observation; sidebands are never wire requests.
pub enum AttachmentTransportItem {
    /// Complete decoded Session protocol frame.
    Session(DecodedFrame),
    /// Validated path observation from the local adapter or Iroh connection.
    Path(v2::LocalSessionTunnelPath),
}

/// One concrete epoch. Implementations retain framing across cancellation of reads.
pub trait AttachmentIo: Send {
    /// Number of already decoded Session frames retained by this epoch.
    fn queued_session_count(&self) -> usize;
    /// Sends the complete Session bytes, translating local envelopes only in the adapter.
    fn write<'a>(&'a mut self, bytes: &'a [u8]) -> TransportFuture<'a, Result<(), ClientError>>;
    /// Reads the next frame or address-free observation.
    fn read(&mut self) -> TransportFuture<'_, Result<AttachmentTransportItem, ClientError>>;
    /// Finishes this epoch's write half.
    fn shutdown(&mut self) -> TransportFuture<'_, Result<(), ClientError>>;
}

/// Opens replacement epochs for an immutable target; concrete paths stay in the adapter.
pub trait AttachmentConnector: Send + Sync {
    /// Opens a bounded authenticated Session stream.
    fn open(
        &self,
        target: ResolvedSessionTarget,
    ) -> TransportFuture<'_, Result<AttachmentTransport, ClientError>>;
}

/// Owns cancellation-safe writes and retirement of one concrete epoch.
pub enum AttachmentTransport {
    /// Retired epoch which cannot transmit more commands.
    Closed,
    /// An active adapter, dropped if a write future is abandoned.
    Open(Box<dyn AttachmentIo>),
}
impl AttachmentTransport {
    /// Wraps a prepared local or authenticated remote adapter.
    pub fn new(io: impl AttachmentIo + 'static) -> Self {
        Self::Open(Box::new(io))
    }
    /// Retained decoded Session frames, used for bounded owner diagnostics.
    pub fn queued_session_count(&self) -> usize {
        match self {
            Self::Closed => 0,
            Self::Open(io) => io.queued_session_count(),
        }
    }
    /// Drops all transport and decoder state for this epoch.
    pub fn invalidate(&mut self) {
        *self = Self::Closed;
    }
    /// Sends one complete command under the standard control budget.
    pub async fn write_session_bytes(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        self.write_until(bytes, control_deadline()).await
    }
    /// Drops an incomplete epoch on cancellation, deadline, or uncertain write failure.
    pub async fn write_until(
        &mut self,
        bytes: &[u8],
        deadline: tokio::time::Instant,
    ) -> Result<(), ClientError> {
        let mut epoch = std::mem::replace(self, Self::Closed);
        let result = if deadline <= tokio::time::Instant::now() {
            Err(control_timeout())
        } else {
            tokio::time::timeout_at(deadline, async {
                match &mut epoch {
                    Self::Closed => Err(attachment_cancelled()),
                    Self::Open(io) => io.write(bytes).await,
                }
            })
            .await
            .unwrap_or_else(|_| Err(control_timeout()))
        };
        // A closed write half may precede the typed outcome on its read half.
        if result.is_ok()
            || result
                .as_ref()
                .err()
                .is_some_and(is_attachment_command_stream_closed)
        {
            *self = epoch;
        }
        result
    }
    /// Reads one decoded item; the concrete adapter owns partial frame bytes.
    pub async fn read_item(&mut self) -> Result<AttachmentTransportItem, ClientError> {
        match self {
            Self::Closed => Err(attachment_cancelled()),
            Self::Open(io) => io.read().await,
        }
    }
    /// Finishes and retires this epoch under the standard control budget.
    pub async fn shutdown(&mut self) -> Result<(), ClientError> {
        let mut epoch = std::mem::replace(self, Self::Closed);
        tokio::time::timeout_at(control_deadline(), async {
            match &mut epoch {
                Self::Closed => Ok(()),
                Self::Open(io) => io.shutdown().await,
            }
        })
        .await
        .unwrap_or_else(|_| Err(control_timeout()))
    }
}
