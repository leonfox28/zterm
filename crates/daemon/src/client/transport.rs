//! Direct and opaque-tunnel byte transport for one attachment epoch.
use super::{
    DEFAULT_DEADLINE, attachment_cancelled, connect_error, daemon_io,
    local_attachment_command_error, local_attachment_io, malformed, service_error,
};
use crate::{
    device_directory::ResolvedSessionTarget, error::DaemonError, service::protocol_error,
    session_wire::FirstFrame,
};
use std::{collections::VecDeque, path::Path};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeroize::Zeroizing;
use zterm_core::DomainErrorKind;
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v2};

pub(super) enum AttachmentTransport {
    Closed,
    Direct {
        stream: tokio::net::UnixStream,
        decoder: FrameDecoder,
        queued: VecDeque<DecodedFrame>,
    },
    Tunnel {
        stream: tokio::net::UnixStream,
        envelope_decoder: FrameDecoder,
        queued_envelopes: VecDeque<DecodedFrame>,
        session_decoder: FrameDecoder,
        queued_session_frames: VecDeque<DecodedFrame>,
        remote_half_closed: bool,
    },
}

pub(super) enum AttachmentTransportItem {
    Session(DecodedFrame),
    Path(v2::LocalSessionTunnelPath),
}

impl AttachmentTransport {
    pub(super) async fn open(
        socket: &Path,
        target: ResolvedSessionTarget,
    ) -> Result<Self, DaemonError> {
        tokio::time::timeout_at(super::control_deadline(), Self::open_inner(socket, target))
            .await
            .map_err(|_| super::control_timeout())?
    }

    async fn open_inner(socket: &Path, target: ResolvedSessionTarget) -> Result<Self, DaemonError> {
        let mut stream = tokio::net::UnixStream::connect(socket)
            .await
            .map_err(connect_error)?;
        let Some(target_device_id) = target.device_id() else {
            return Ok(Self::Direct {
                stream,
                decoder: FrameDecoder::new(),
                queued: VecDeque::new(),
            });
        };

        let request_id = 1;
        let open = encode_message(
            WireKind::LocalSessionTunnelOpenRequest,
            request_id,
            u32::try_from(DEFAULT_DEADLINE.as_millis()).unwrap_or(u32::MAX),
            &v2::LocalSessionTunnelOpenRequest {
                protocol_version: zterm_proto::LOCAL_SESSION_TUNNEL_VERSION,
                target_device_id: Some(target_device_id.into()),
            },
        )
        .map_err(protocol_error)?;
        stream
            .write_all(&open)
            .await
            .map_err(|error| daemon_io("write local Session tunnel Open", error))?;

        let first = tokio::time::timeout(DEFAULT_DEADLINE, read_tunnel_first(&mut stream))
            .await
            .map_err(|_| {
                DaemonError::new(
                    DomainErrorKind::DeadlineExceeded,
                    "timed out opening remote Session tunnel",
                )
            })??;
        if first.frame.kind == WireKind::ServiceErrorResponse {
            if first.frame.request_id != request_id {
                return Err(malformed(
                    "remote Session tunnel error correlation mismatch",
                ));
            }
            return Err(service_error(&first.frame)?);
        }
        if first.frame.kind != WireKind::LocalSessionTunnelOpened
            || first.frame.request_id != request_id
        {
            return Err(malformed(
                "remote Session tunnel Opened correlation mismatch",
            ));
        }
        let opened: v2::LocalSessionTunnelOpened = first
            .frame
            .decode_message(WireKind::LocalSessionTunnelOpened)
            .map_err(protocol_error)?;
        if opened.protocol_version != zterm_proto::LOCAL_SESSION_TUNNEL_VERSION {
            return Err(DaemonError::new(
                DomainErrorKind::WireMajorMismatch,
                "remote Session tunnel returned an unsupported protocol version",
            ));
        }
        Ok(Self::Tunnel {
            stream,
            envelope_decoder: first.decoder,
            queued_envelopes: first.queued,
            session_decoder: FrameDecoder::new(),
            queued_session_frames: VecDeque::new(),
            remote_half_closed: false,
        })
    }

    pub(super) fn queued_session_count(&self) -> usize {
        match self {
            Self::Closed => 0,
            Self::Direct { queued, .. } => queued.len(),
            Self::Tunnel {
                queued_session_frames,
                ..
            } => queued_session_frames.len(),
        }
    }

    pub(super) async fn write_session_bytes(&mut self, bytes: &[u8]) -> Result<(), DaemonError> {
        self.write_until(bytes, super::control_deadline()).await
    }

    pub(super) fn invalidate(&mut self) {
        *self = Self::Closed;
    }

    pub(super) async fn write_until(
        &mut self,
        bytes: &[u8],
        deadline: tokio::time::Instant,
    ) -> Result<(), DaemonError> {
        // Move the epoch into the write future. Cancellation drops its socket;
        // only a complete write can restore it for subsequent commands.
        let mut epoch = std::mem::replace(self, Self::Closed);
        let result = if deadline <= tokio::time::Instant::now() {
            Err(super::control_timeout())
        } else {
            tokio::time::timeout_at(deadline, epoch.write_inner(bytes))
                .await
                .unwrap_or_else(|_| Err(super::control_timeout()))
        };
        // A peer may close its read half before delivering a typed terminal
        // outcome. Preserve only that bounded read-drain opportunity; the
        // Session client rejects all subsequent writes on this epoch.
        if result.is_ok()
            || result
                .as_ref()
                .err()
                .is_some_and(super::is_attachment_command_stream_closed)
        {
            *self = epoch;
        }
        result
    }

    async fn write_inner(&mut self, bytes: &[u8]) -> Result<(), DaemonError> {
        match self {
            Self::Closed => Err(attachment_cancelled()),
            Self::Direct { stream, .. } => stream
                .write_all(bytes)
                .await
                .map_err(local_attachment_command_error),
            Self::Tunnel { stream, .. } => {
                for chunk in bytes.chunks(zterm_proto::MAX_LOCAL_SESSION_TUNNEL_DATA_BYTES) {
                    let envelope = encode_message(
                        WireKind::LocalSessionTunnelData,
                        0,
                        0,
                        &v2::LocalSessionTunnelData {
                            bytes: chunk.to_vec(),
                        },
                    )
                    .map_err(protocol_error)?;
                    stream
                        .write_all(&envelope)
                        .await
                        .map_err(local_attachment_command_error)?;
                }
                Ok(())
            }
        }
    }

    pub(super) async fn read_item(&mut self) -> Result<AttachmentTransportItem, DaemonError> {
        match self {
            Self::Closed => Err(attachment_cancelled()),
            Self::Direct {
                stream,
                decoder,
                queued,
            } => read_frame_parts(stream, decoder, queued)
                .await
                .map(AttachmentTransportItem::Session),
            Self::Tunnel {
                stream,
                envelope_decoder,
                queued_envelopes,
                session_decoder,
                queued_session_frames,
                remote_half_closed,
            } => {
                if let Some(frame) = queued_session_frames.pop_front() {
                    return Ok(AttachmentTransportItem::Session(frame));
                }
                loop {
                    let envelope =
                        read_tunnel_frame_parts(stream, envelope_decoder, queued_envelopes).await?;
                    if envelope.request_id != 0 || envelope.deadline_ms != 0 {
                        return Err(malformed(
                            "remote Session tunnel stream frame used a request ID or deadline",
                        ));
                    }
                    match envelope.kind {
                        WireKind::LocalSessionTunnelData => {
                            if *remote_half_closed {
                                return Err(malformed(
                                    "remote Session tunnel returned Data after HalfClose",
                                ));
                            }
                            let data: v2::LocalSessionTunnelData = envelope
                                .decode_message(WireKind::LocalSessionTunnelData)
                                .map_err(protocol_error)?;
                            validate_tunnel_data(&data.bytes)?;
                            queued_session_frames
                                .extend(session_decoder.feed(&data.bytes).map_err(protocol_error)?);
                            if let Some(frame) = queued_session_frames.pop_front() {
                                return Ok(AttachmentTransportItem::Session(frame));
                            }
                        }
                        WireKind::LocalSessionTunnelPath => {
                            let path: v2::LocalSessionTunnelPath = envelope
                                .decode_message(WireKind::LocalSessionTunnelPath)
                                .map_err(protocol_error)?;
                            validate_tunnel_path(&path)?;
                            return Ok(AttachmentTransportItem::Path(path));
                        }
                        WireKind::LocalSessionTunnelHalfClose => {
                            if *remote_half_closed {
                                return Err(malformed(
                                    "remote Session tunnel returned HalfClose more than once",
                                ));
                            }
                            let _: v2::LocalSessionTunnelHalfClose = envelope
                                .decode_message(WireKind::LocalSessionTunnelHalfClose)
                                .map_err(protocol_error)?;
                            *remote_half_closed = true;
                        }
                        WireKind::LocalSessionTunnelClosed => {
                            let closed: v2::LocalSessionTunnelClosed = envelope
                                .decode_message(WireKind::LocalSessionTunnelClosed)
                                .map_err(protocol_error)?;
                            let reason = v2::LocalSessionTunnelCloseReason::try_from(closed.reason)
                                .map_err(|_| {
                                    malformed("unknown remote Session tunnel close reason")
                                })?;
                            if reason == v2::LocalSessionTunnelCloseReason::RemoteEof {
                                if !*remote_half_closed {
                                    return Err(malformed(
                                        "remote Session tunnel reported RemoteEof without HalfClose",
                                    ));
                                }
                                std::mem::replace(session_decoder, FrameDecoder::new())
                                    .finish()
                                    .map_err(protocol_error)?;
                            } else {
                                // A non-clean tunnel loss may split an otherwise valid inner
                                // Session frame. Discard that epoch's decoder state so the close
                                // reason remains retryable instead of being masked as malformed.
                                *session_decoder = FrameDecoder::new();
                            }
                            return Err(tunnel_closed(reason));
                        }
                        _ => {
                            return Err(malformed("invalid envelope from remote Session tunnel"));
                        }
                    }
                }
            }
        }
    }

    pub(super) async fn shutdown(&mut self) -> Result<(), DaemonError> {
        let result = tokio::time::timeout_at(super::control_deadline(), self.shutdown_inner())
            .await
            .unwrap_or_else(|_| Err(super::control_timeout()));
        self.invalidate();
        result
    }

    async fn shutdown_inner(&mut self) -> Result<(), DaemonError> {
        match self {
            Self::Closed => Ok(()),
            Self::Direct { stream, .. } => stream
                .shutdown()
                .await
                .map_err(|error| local_attachment_io("finish local terminal detach", error)),
            Self::Tunnel { stream, .. } => {
                let half_close = encode_message(
                    WireKind::LocalSessionTunnelHalfClose,
                    0,
                    0,
                    &v2::LocalSessionTunnelHalfClose {},
                )
                .map_err(protocol_error)?;
                stream
                    .write_all(&half_close)
                    .await
                    .map_err(local_attachment_command_error)?;
                stream.shutdown().await.map_err(|error| {
                    local_attachment_io("finish remote Session tunnel detach", error)
                })
            }
        }
    }
}

fn validate_tunnel_data(bytes: &[u8]) -> Result<(), DaemonError> {
    if bytes.is_empty() {
        return Err(malformed("remote Session tunnel returned empty Data"));
    }
    if bytes.len() > zterm_proto::MAX_LOCAL_SESSION_TUNNEL_DATA_BYTES {
        return Err(DaemonError::new(
            DomainErrorKind::ControlPayloadTooLarge,
            "remote Session tunnel Data exceeds the chunk limit",
        ));
    }
    Ok(())
}

fn validate_tunnel_path(path: &v2::LocalSessionTunnelPath) -> Result<(), DaemonError> {
    match v2::TerminalConnectionPath::try_from(path.path) {
        Ok(v2::TerminalConnectionPath::Unknown)
        | Ok(v2::TerminalConnectionPath::Direct)
        | Ok(v2::TerminalConnectionPath::Relay) => Ok(()),
        Ok(v2::TerminalConnectionPath::Unspecified) | Err(_) => {
            Err(malformed("remote Session tunnel returned an unknown path"))
        }
    }
}

fn tunnel_closed(reason: v2::LocalSessionTunnelCloseReason) -> DaemonError {
    match reason {
        v2::LocalSessionTunnelCloseReason::RemoteEof => attachment_cancelled(),
        v2::LocalSessionTunnelCloseReason::TransportLost
        | v2::LocalSessionTunnelCloseReason::DaemonStopping => DaemonError::new(
            DomainErrorKind::TransportUnavailable,
            "remote Session tunnel closed",
        ),
        v2::LocalSessionTunnelCloseReason::ProtocolError
        | v2::LocalSessionTunnelCloseReason::Unspecified => {
            malformed("remote Session tunnel closed with a protocol error")
        }
    }
}

pub(super) async fn read_tunnel_first(
    stream: &mut tokio::net::UnixStream,
) -> Result<FirstFrame, DaemonError> {
    let mut decoder = FrameDecoder::new();
    let mut buffer = Zeroizing::new([0_u8; 16 * 1024]);
    loop {
        let read = stream
            .read(&mut *buffer)
            .await
            .map_err(|error| daemon_io("read remote Session tunnel Opened", error))?;
        if read == 0 {
            // The viewer daemon may have stopped between arbitrary Unix-socket
            // writes. A truncated outer frame is therefore a lost transport
            // epoch, not evidence that the peer emitted malformed bytes.
            return Err(DaemonError::new(
                DomainErrorKind::TransportUnavailable,
                "remote Session tunnel closed before Opened",
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

async fn read_tunnel_frame_parts(
    stream: &mut tokio::net::UnixStream,
    decoder: &mut FrameDecoder,
    queued: &mut VecDeque<DecodedFrame>,
) -> Result<DecodedFrame, DaemonError> {
    if let Some(frame) = queued.pop_front() {
        return Ok(frame);
    }
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|error| daemon_io("read remote Session tunnel", error))?;
        if read == 0 {
            // Discard any partial outer envelope from the dead viewer-daemon
            // epoch. Reconnect must not be suppressed by a synthetic framing
            // error caused only by process interruption.
            *decoder = FrameDecoder::new();
            queued.clear();
            return Err(DaemonError::new(
                DomainErrorKind::TransportUnavailable,
                "remote Session tunnel closed without a terminal outcome",
            ));
        }
        queued.extend(decoder.feed(&buffer[..read]).map_err(protocol_error)?);
        if let Some(frame) = queued.pop_front() {
            return Ok(frame);
        }
    }
}

pub(super) async fn read_frame_parts(
    stream: &mut tokio::net::UnixStream,
    decoder: &mut FrameDecoder,
    queued: &mut VecDeque<DecodedFrame>,
) -> Result<DecodedFrame, DaemonError> {
    if let Some(frame) = queued.pop_front() {
        return Ok(frame);
    }
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|error| daemon_io("read local terminal event", error))?;
        if read == 0 {
            std::mem::replace(decoder, FrameDecoder::new())
                .finish()
                .map_err(protocol_error)?;
            return Err(attachment_cancelled());
        }
        queued.extend(decoder.feed(&buffer[..read]).map_err(protocol_error)?);
        if let Some(frame) = queued.pop_front() {
            return Ok(frame);
        }
    }
}
