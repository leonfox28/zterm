//! One bounded uploader shared by desktop and mobile. Never retries a transfer.

use crate::{
    error::ClientError,
    framing::FrameReader,
    model::ResolvedSessionTarget,
    protocol::{DEFAULT_DEADLINE, malformed, protocol_error, service_error},
    transport::TransportFuture,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::watch,
    time::{Instant, timeout, timeout_at},
};
use tokio_util::sync::CancellationToken;
use zterm_core::{Capabilities, DomainErrorKind, upload::*};
use zterm_proto::{DecodedFrame, WireKind, upload::UploadMessage};

/// Independently readable half; permits progress reads while a write is blocked.
pub trait UploadReader: Send {
    /// Reads the next service frame, retaining incomplete reads across cancellation.
    fn read(&mut self) -> TransportFuture<'_, Result<DecodedFrame, ClientError>>;
}
/// Independently writable half; adapters own local tunnel envelopes.
pub trait UploadWriter: Send {
    /// Sends one complete encoded service frame.
    fn write<'a>(&'a mut self, bytes: &'a [u8]) -> TransportFuture<'a, Result<(), ClientError>>;
}

/// Framed reader retaining the connection demand/admission for its lifetime.
pub struct FramedUploadReader<R> {
    frames: FrameReader<R>,
    _guard: Box<dyn Send>,
}
impl<R: AsyncRead + Unpin> FramedUploadReader<R> {
    /// Wraps a fresh raw stream and its adapter-owned admission guard.
    pub fn new(reader: R, guard: impl Send + 'static) -> Self {
        Self {
            frames: FrameReader::new(reader),
            _guard: Box::new(guard),
        }
    }
}
impl<R: AsyncRead + Unpin + Send> UploadReader for FramedUploadReader<R> {
    fn read(&mut self) -> TransportFuture<'_, Result<DecodedFrame, ClientError>> {
        Box::pin(self.frames.read())
    }
}

/// Raw writer for authenticated transports without local envelopes.
pub struct AsyncUploadWriter<W>(pub W);
impl<W: AsyncWrite + Unpin + Send> UploadWriter for AsyncUploadWriter<W> {
    fn write<'a>(&'a mut self, bytes: &'a [u8]) -> TransportFuture<'a, Result<(), ClientError>> {
        Box::pin(async move {
            self.0.write_all(bytes).await.map_err(|_| {
                ClientError::new(
                    DomainErrorKind::TransportUnavailable,
                    "upload stream write failed",
                )
            })
        })
    }
}

/// Capability metadata and independently owned halves for exactly one attempt.
pub struct UploadConnection {
    /// Negotiated on the connection actually serving this stream.
    pub capabilities: Capabilities,
    /// Progress and result reader.
    pub reader: Box<dyn UploadReader>,
    /// File-data writer.
    pub writer: Box<dyn UploadWriter>,
}

/// Platform adapter opening a separate service stream to a frozen target.
pub trait UploadConnector: Send + Sync {
    /// Opens one stream; never sends UploadBegin on a peer lacking the capability.
    fn open_upload(
        &self,
        target: ResolvedSessionTarget,
    ) -> TransportFuture<'_, Result<UploadConnection, ClientError>>;
}

/// Frozen destination captured by the existing terminal driver before preparation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadOrigin {
    /// Resolved host; display aliases never participate in transfer routing.
    pub target: ResolvedSessionTarget,
    /// Exact session and current attachment.
    pub binding: UploadBinding,
}

/// Latest preparation state used before source acquisition.
#[must_use]
pub const fn preparing() -> UploadProgress {
    UploadProgress {
        phase: UploadPhase::Preparing,
        accepted_bytes: 0,
        total_bytes: None,
    }
}

/// Encodes one validated upload path as a plain paste, with no modifiers or Enter.
#[must_use]
pub fn paste_bytes(file: &UploadedFile, modes: &zterm_core::terminal::TerminalModes) -> Vec<u8> {
    if modes.bracketed_paste {
        [
            b"\x1b[200~".as_slice(),
            file.path().as_bytes(),
            b"\x1b[201~".as_slice(),
        ]
        .concat()
    } else {
        file.path().as_bytes().to_vec()
    }
}

/// Uploads one prepared readable source. Cancellation drops both independent halves.
pub async fn upload<R: AsyncRead + Unpin + Send>(
    connector: &dyn UploadConnector,
    target: ResolvedSessionTarget,
    binding: UploadBinding,
    metadata: UploadMetadata,
    source: R,
    progress: &watch::Sender<UploadProgress>,
    cancel: &CancellationToken,
) -> Result<UploadedFile, ClientError> {
    let mut connection = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(cancelled()),
        result = timeout(DEFAULT_DEADLINE, connector.open_upload(target)) => result.map_err(|_| timed_out())??,
    };
    if !connection
        .capabilities
        .contains(Capabilities::FILE_UPLOAD_SERVICE)
    {
        return Err(ClientError::new(
            DomainErrorKind::ServiceNotImplemented,
            "remote host or local daemon needs an upgrade to support file uploads",
        ));
    }
    let ready = async {
        send(
            &mut *connection.writer,
            UploadMessage::Begin {
                binding,
                metadata: metadata.clone(),
            },
        )
        .await?;
        match receive(&mut *connection.reader).await? {
            UploadMessage::Ready { id, size } if size == metadata.size() => Ok(id),
            _ => Err(malformed("unexpected upload admission response")),
        }
    };
    let id = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(cancelled()),
        result = timeout(DEFAULT_DEADLINE, ready) => result.map_err(|_| timed_out())??,
    };
    let operation = transfer(&mut connection, id, metadata.size(), source, progress);
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(cancelled()),
        result = operation => result,
    };
    if result
        .as_ref()
        .is_err_and(|error| error.kind() == DomainErrorKind::Cancelled)
    {
        // Best effort only. Stream retirement also removes staging; a committed
        // file is deliberately retained even if cancellation raced publication.
        let _ = timeout(
            DEFAULT_DEADLINE,
            send(&mut *connection.writer, UploadMessage::Cancel { id }),
        )
        .await;
    }
    result
}

async fn transfer<R: AsyncRead + Unpin + Send>(
    connection: &mut UploadConnection,
    id: TransferId,
    size: u64,
    mut source: R,
    progress: &watch::Sender<UploadProgress>,
) -> Result<UploadedFile, ClientError> {
    report(progress, UploadPhase::Uploading, 0, size);
    let sent = Arc::new(AtomicU64::new(0));
    let (accepted, mut credit) = watch::channel(0_u64);
    let writer = &mut *connection.writer;
    let reader = &mut *connection.reader;
    let writing = async {
        let mut offset = 0;
        let mut bytes = vec![0_u8; UPLOAD_CHUNK_BYTES];
        while offset < size {
            while offset.saturating_sub(*credit.borrow_and_update()) >= UPLOAD_WINDOW_BYTES {
                timeout(UPLOAD_IDLE_TIMEOUT, credit.changed())
                    .await
                    .map_err(|_| timed_out())?
                    .map_err(|_| cancelled())?;
            }
            let available = UPLOAD_WINDOW_BYTES - offset.saturating_sub(*credit.borrow());
            let wanted = usize::try_from(
                (size - offset)
                    .min(UPLOAD_CHUNK_BYTES as u64)
                    .min(available),
            )
            .expect("bounded chunk length");
            let length = timeout(UPLOAD_IDLE_TIMEOUT, source.read(&mut bytes[..wanted]))
                .await
                .map_err(|_| timed_out())?
                .map_err(|_| source_invalid())?;
            if length == 0 {
                return Err(source_invalid());
            }
            sent.store(offset + length as u64, Ordering::Release);
            timeout(
                UPLOAD_IDLE_TIMEOUT,
                send(
                    writer,
                    UploadMessage::Chunk {
                        id,
                        offset,
                        data: bytes[..length].to_vec(),
                    },
                ),
            )
            .await
            .map_err(|_| timed_out())??;
            offset += length as u64;
        }
        let length = timeout(UPLOAD_IDLE_TIMEOUT, source.read(&mut bytes[..1]))
            .await
            .map_err(|_| timed_out())?
            .map_err(|_| source_invalid())?;
        if length != 0 {
            return Err(source_invalid());
        }
        Ok::<_, ClientError>(())
    };
    let reading = async {
        let mut previous = 0;
        let mut deadline = Instant::now() + UPLOAD_IDLE_TIMEOUT;
        while previous < size {
            match timeout_at(deadline, receive(reader))
                .await
                .map_err(|_| timed_out())??
            {
                UploadMessage::Progress {
                    id: received,
                    accepted_bytes,
                } if received == id
                    && accepted_bytes >= previous
                    && accepted_bytes <= sent.load(Ordering::Acquire) =>
                {
                    if accepted_bytes > previous {
                        previous = accepted_bytes;
                        accepted.send_replace(previous);
                        report(progress, UploadPhase::Uploading, previous, size);
                        deadline = Instant::now() + UPLOAD_IDLE_TIMEOUT;
                    }
                }
                _ => return Err(malformed("invalid upload progress")),
            }
        }
        Ok::<_, ClientError>(())
    };
    tokio::try_join!(writing, reading)?;
    report(progress, UploadPhase::Finishing, size, size);
    let finish = async {
        send(&mut *connection.writer, UploadMessage::Finish { id, size }).await?;
        match receive(&mut *connection.reader).await? {
            UploadMessage::Completed {
                id: completed,
                file,
            } if completed == id && file.size() == size => Ok(file),
            _ => Err(malformed("unexpected upload publication response")),
        }
    };
    let file = timeout(DEFAULT_DEADLINE, finish)
        .await
        .unwrap_or_else(|_| Err(timed_out()))
        .map_err(|error| {
            if matches!(
                error.kind(),
                DomainErrorKind::TransportUnavailable | DomainErrorKind::DeadlineExceeded
            ) {
                ClientError::new(
                    DomainErrorKind::UploadOutcomeUnknown,
                    "upload result was lost; a remote file may exist; retry creates a new file",
                )
            } else {
                error
            }
        })?;
    report(progress, UploadPhase::Completed, size, size);
    Ok(file)
}

async fn send(writer: &mut dyn UploadWriter, message: UploadMessage) -> Result<(), ClientError> {
    writer
        .write(&message.encode(1).map_err(protocol_error)?)
        .await
}
async fn receive(reader: &mut dyn UploadReader) -> Result<UploadMessage, ClientError> {
    let frame = reader.read().await?;
    if frame.request_id != 1 || frame.deadline_ms != 0 {
        return Err(malformed("upload response correlation mismatch"));
    }
    if frame.kind == WireKind::ServiceErrorResponse {
        return Err(service_error(&frame)?);
    }
    UploadMessage::decode(&frame).map_err(protocol_error)
}
fn report(
    sender: &watch::Sender<UploadProgress>,
    phase: UploadPhase,
    accepted_bytes: u64,
    size: u64,
) {
    sender.send_replace(UploadProgress {
        phase,
        accepted_bytes,
        total_bytes: Some(size),
    });
}
fn cancelled() -> ClientError {
    ClientError::new(DomainErrorKind::Cancelled, "upload cancelled")
}
fn timed_out() -> ClientError {
    ClientError::new(
        DomainErrorKind::DeadlineExceeded,
        "upload made no progress within its deadline",
    )
}
fn source_invalid() -> ClientError {
    ClientError::new(
        DomainErrorKind::UploadSourceInvalid,
        "upload source is unreadable or changed size",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use zterm_core::{AttachmentId, SessionId};

    struct Connector(Mutex<Option<UploadConnection>>);
    impl UploadConnector for Connector {
        fn open_upload(
            &self,
            _: ResolvedSessionTarget,
        ) -> TransportFuture<'_, Result<UploadConnection, ClientError>> {
            Box::pin(async move {
                Ok(self
                    .0
                    .lock()
                    .expect("upload fixture")
                    .take()
                    .expect("a transfer must never reopen or replay"))
            })
        }
    }
    fn connection(capabilities: u64) -> (Connector, tokio::io::DuplexStream) {
        let (client, server) = tokio::io::duplex(1024);
        let (reader, writer) = tokio::io::split(client);
        (
            Connector(Mutex::new(Some(UploadConnection {
                capabilities: Capabilities::from_bits_retain(capabilities),
                reader: Box::new(FramedUploadReader::new(reader, ())),
                writer: Box::new(AsyncUploadWriter(writer)),
            }))),
            server,
        )
    }
    fn binding() -> UploadBinding {
        UploadBinding {
            session_id: SessionId::from_array([1; 16]),
            attachment_id: AttachmentId::from_array([2; 16]),
        }
    }

    #[tokio::test]
    async fn old_host_receives_no_upload_bytes() {
        let (connector, mut server) = connection(0);
        let (progress, _) = watch::channel(preparing());
        let error = upload(
            &connector,
            ResolvedSessionTarget::local(),
            binding(),
            UploadMetadata::new(0, "").expect("upload fixture"),
            [].as_slice(),
            &progress,
            &CancellationToken::new(),
        )
        .await
        .expect_err("upload must fail");
        assert_eq!(error.kind(), DomainErrorKind::ServiceNotImplemented);
        assert_eq!(server.read(&mut [0; 1]).await.expect("upload fixture"), 0);
    }

    #[tokio::test]
    async fn lost_publication_reply_is_unknown_without_replay() {
        let (connector, server) = connection(Capabilities::FILE_UPLOAD_SERVICE);
        let host = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = FrameReader::new(reader);
            assert!(matches!(
                UploadMessage::decode(&reader.read().await.expect("upload fixture"))
                    .expect("upload fixture"),
                UploadMessage::Begin { .. }
            ));
            let id = TransferId::from_array([3; 16]);
            writer
                .write_all(
                    &UploadMessage::Ready { id, size: 3 }
                        .encode(1)
                        .expect("upload fixture"),
                )
                .await
                .expect("upload fixture");
            assert!(
                matches!(UploadMessage::decode(&reader.read().await.expect("upload fixture")).expect("upload fixture"), UploadMessage::Chunk { data, .. } if data == b"abc")
            );
            writer
                .write_all(
                    &UploadMessage::Progress {
                        id,
                        accepted_bytes: 3,
                    }
                    .encode(1)
                    .expect("upload fixture"),
                )
                .await
                .expect("upload fixture");
            assert!(matches!(
                UploadMessage::decode(&reader.read().await.expect("upload fixture"))
                    .expect("upload fixture"),
                UploadMessage::Finish { .. }
            ));
            // Publication occurred; deliberately lose its response.
        });
        let (progress, _) = watch::channel(preparing());
        let error = upload(
            &connector,
            ResolvedSessionTarget::local(),
            binding(),
            UploadMetadata::new(3, "").expect("upload fixture"),
            b"abc".as_slice(),
            &progress,
            &CancellationToken::new(),
        )
        .await
        .expect_err("upload must fail");
        assert_eq!(error.kind(), DomainErrorKind::UploadOutcomeUnknown);
        assert_eq!(progress.borrow().phase, UploadPhase::Finishing);
        host.await.expect("upload fixture");
    }
}
