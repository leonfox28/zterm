//! Bounded first-frame decoding with retained stream leftovers.
use crate::{error::ClientError, protocol::protocol_error};
use std::collections::VecDeque;
use tokio::io::{AsyncRead, AsyncReadExt};
use zeroize::Zeroizing;
use zterm_core::DomainErrorKind;
use zterm_proto::{DecodedFrame, FrameDecoder};

/// One decoded first frame plus the single decoder's retained leftovers.
pub struct FirstFrame {
    /// First complete frame.
    pub frame: DecodedFrame,
    /// Decoder holding any partial following frame.
    pub decoder: FrameDecoder,
    /// Complete coalesced frames following the first.
    pub queued: VecDeque<DecodedFrame>,
}

/// Cancellation-safe framed reader reused by independent streaming services.
pub struct FrameReader<R> {
    reader: R,
    decoder: FrameDecoder,
    queued: VecDeque<DecodedFrame>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    /// Starts framing a fresh stream.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            decoder: FrameDecoder::new(),
            queued: VecDeque::new(),
        }
    }

    /// Continues after a dispatcher consumed the first frame.
    pub fn after_first(reader: R, first: FirstFrame) -> Self {
        Self {
            reader,
            decoder: first.decoder,
            queued: first.queued,
        }
    }

    /// Reads one frame, retaining partial and coalesced bytes across cancellation.
    pub async fn read(&mut self) -> Result<DecodedFrame, ClientError> {
        loop {
            if let Some(frame) = self.queued.pop_front() {
                return Ok(frame);
            }
            let mut bytes = [0_u8; 16 * 1024];
            let length = self.reader.read(&mut bytes).await.map_err(|_| {
                ClientError::new(
                    DomainErrorKind::TransportUnavailable,
                    "service stream read failed",
                )
            })?;
            if length == 0 {
                return Err(ClientError::new(
                    DomainErrorKind::TransportUnavailable,
                    "service stream closed",
                ));
            }
            self.queued.extend(
                self.decoder
                    .feed(&bytes[..length])
                    .map_err(protocol_error)?,
            );
        }
    }
}

/// Reads exactly through the first complete frame while retaining decoder
/// state and any additional frames received by the same bounded read.
pub async fn read_first<Reader>(reader: &mut Reader) -> Result<FirstFrame, ClientError>
where
    Reader: AsyncRead + Unpin,
{
    let mut decoder = FrameDecoder::new();
    let mut buffer = Zeroizing::new([0_u8; 16 * 1024]);
    loop {
        let read = reader
            .read(&mut *buffer)
            .await
            .map_err(|error| daemon_io("read Session request", error))?;
        if read == 0 {
            decoder.finish().map_err(protocol_error)?;
            return Err(ClientError::new(
                DomainErrorKind::Cancelled,
                "client closed before sending a Session request",
            ));
        }
        let mut frames = VecDeque::from(decoder.feed(&buffer[..read]).map_err(protocol_error)?);
        if let Some(frame) = frames.pop_front() {
            return Ok(FirstFrame {
                frame,
                decoder,
                queued: frames,
            });
        }
    }
}

fn daemon_io(operation: &str, error: std::io::Error) -> ClientError {
    ClientError::new(
        DomainErrorKind::DaemonStopped,
        format!("{operation}: {error}"),
    )
}
