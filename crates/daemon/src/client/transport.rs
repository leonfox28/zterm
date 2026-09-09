//! Direct and opaque-tunnel byte transport for one attachment epoch.
use std::{collections::VecDeque, path::Path, sync::Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeroize::Zeroizing;
use zterm_client::progress::{LocalProgressDecoder, ProgressObserver};
use zterm_client::transport::{
    AttachmentConnector, AttachmentIo, AttachmentTransport, AttachmentTransportItem,
    TransportFuture,
};
use zterm_client::{
    error::ClientError as DaemonError,
    framing::FirstFrame,
    model::ResolvedSessionTarget,
    protocol::{DEFAULT_DEADLINE, attachment_cancelled, malformed, protocol_error, service_error},
};
use zterm_core::DomainErrorKind;
use zterm_core::connection_progress::ConnectionStage;
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v2};

/// Concrete same-UID Session adapter, including opaque remote envelopes.
#[allow(missing_docs)]
pub(super) enum UnixAttachmentTransport {
    Direct {
        stream: tokio::net::UnixStream,
        decoder: FrameDecoder,
        queued: VecDeque<DecodedFrame>,
    },
    Tunnel {
        stream: tokio::net::UnixStream,
        remote_capabilities: zterm_core::Capabilities,
        decoder: TunnelDecoder,
    },
}

impl UnixAttachmentTransport {
    pub(super) async fn open(
        socket: &Path,
        target: ResolvedSessionTarget,
        progress: &ProgressObserver,
    ) -> Result<Self, DaemonError> {
        tokio::time::timeout_at(
            zterm_client::protocol::control_deadline(),
            Self::open_inner(socket, target, progress),
        )
        .await
        .map_err(|_| zterm_client::protocol::control_timeout())?
    }

    async fn open_inner(
        socket: &Path,
        target: ResolvedSessionTarget,
        progress: &ProgressObserver,
    ) -> Result<Self, DaemonError> {
        progress.report(ConnectionStage::OpeningLocalChannel);
        let mut stream = tokio::net::UnixStream::connect(socket)
            .await
            .map_err(connect_error)?;
        let Some(target_device_id) = target.device_id() else {
            progress.report(ConnectionStage::SessionChannelReady);
            return Ok(Self::Direct {
                stream,
                decoder: FrameDecoder::new(),
                queued: VecDeque::new(),
            });
        };

        progress.report(ConnectionStage::OpeningRemoteChannel);
        let request_id = 1;
        let open = encode_message(
            WireKind::LocalSessionTunnelOpenRequest,
            request_id,
            u32::try_from(DEFAULT_DEADLINE.as_millis()).unwrap_or(u32::MAX),
            &v2::LocalSessionTunnelOpenRequest {
                report_progress: progress.is_active(),
                protocol_version: zterm_proto::LOCAL_SESSION_TUNNEL_VERSION,
                target_device_id: Some(target_device_id.into()),
            },
        )
        .map_err(protocol_error)?;
        stream
            .write_all(&open)
            .await
            .map_err(|error| daemon_io("write local Session tunnel Open", error))?;

        let first = tokio::time::timeout(
            DEFAULT_DEADLINE,
            read_tunnel_first(&mut stream, progress.clone()),
        )
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
            remote_capabilities: zterm_core::Capabilities::from_bits_retain(
                opened.remote_capabilities.unwrap_or_default(),
            ),
            decoder: TunnelDecoder {
                envelope_decoder: first.decoder,
                queued_envelopes: first.queued,
                session_decoder: FrameDecoder::new(),
                queued_session_frames: VecDeque::new(),
                remote_half_closed: false,
            },
        })
    }

    pub(super) fn queued_session_count(&self) -> usize {
        match self {
            Self::Direct { queued, .. } => queued.len(),
            Self::Tunnel { decoder, .. } => decoder.queued_session_frames.len(),
        }
    }

    async fn write_inner(&mut self, bytes: &[u8]) -> Result<(), DaemonError> {
        match self {
            Self::Direct { stream, .. } => stream
                .write_all(bytes)
                .await
                .map_err(local_attachment_command_error),
            Self::Tunnel { stream, .. } => write_tunnel_bytes(stream, bytes).await,
        }
    }

    #[cfg(test)]
    #[allow(dead_code)] // Used by the shared client's included desktop-adapter fixtures.
    pub(super) fn tunnel_after_first(
        stream: tokio::net::UnixStream,
        first: Option<FirstFrame>,
    ) -> Self {
        let decoder = first.map_or_else(TunnelDecoder::default, |first| TunnelDecoder {
            envelope_decoder: first.decoder,
            queued_envelopes: first.queued,
            ..TunnelDecoder::default()
        });
        Self::Tunnel {
            stream,
            remote_capabilities: zterm_core::Capabilities::default(),
            decoder,
        }
    }

    pub(super) async fn read_item(&mut self) -> Result<AttachmentTransportItem, DaemonError> {
        match self {
            Self::Direct {
                stream,
                decoder,
                queued,
            } => read_frame_parts(stream, decoder, queued)
                .await
                .map(AttachmentTransportItem::Session),
            Self::Tunnel {
                stream, decoder, ..
            } => decoder.read_item(stream).await,
        }
    }

    async fn shutdown_inner(&mut self) -> Result<(), DaemonError> {
        match self {
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

async fn write_tunnel_bytes<W: tokio::io::AsyncWrite + Unpin>(
    stream: &mut W,
    bytes: &[u8],
) -> Result<(), DaemonError> {
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

#[derive(Default)]
pub(super) struct TunnelDecoder {
    envelope_decoder: FrameDecoder,
    queued_envelopes: VecDeque<DecodedFrame>,
    session_decoder: FrameDecoder,
    queued_session_frames: VecDeque<DecodedFrame>,
    remote_half_closed: bool,
}
impl TunnelDecoder {
    async fn read_item<R: tokio::io::AsyncRead + Unpin>(
        &mut self,
        stream: &mut R,
    ) -> Result<AttachmentTransportItem, DaemonError> {
        let Self {
            envelope_decoder,
            queued_envelopes,
            session_decoder,
            queued_session_frames,
            remote_half_closed,
        } = self;
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
                        .map_err(|_| malformed("unknown remote Session tunnel close reason"))?;
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

/// Reads a tunnel Open result, retaining coalesced envelope bytes.
pub(super) async fn read_tunnel_first(
    stream: &mut tokio::net::UnixStream,
    progress: ProgressObserver,
) -> Result<FirstFrame, DaemonError> {
    let mut progress = LocalProgressDecoder::new(1, progress);
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
        while let Some(frame) = frames.pop_front() {
            if progress.consume(&frame)? {
                continue;
            }
            return Ok(FirstFrame {
                frame,
                decoder,
                queued: frames,
            });
        }
    }
}

async fn read_tunnel_frame_parts<R: tokio::io::AsyncRead + Unpin>(
    stream: &mut R,
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

/// Reads one local frame while preserving partial and coalesced bytes.
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

/// Frozen desktop socket factory; aliases have already been resolved by the daemon.
pub(crate) struct UnixAttachmentConnector {
    progress: ProgressObserver,
    socket: std::path::PathBuf,
    remote_restarter: Option<Arc<dyn RemoteDaemonRestarter>>,
}

/// Desktop-only capability for restoring the viewer daemon of a remote Session.
/// Implementations use the ordinary lifecycle lock; local Sessions never restart.
pub trait RemoteDaemonRestarter: Send + Sync {
    /// Ensures the configured local daemon is available for a replacement tunnel.
    fn ensure_running(&self) -> TransportFuture<'_, Result<(), DaemonError>>;
}

impl UnixAttachmentConnector {
    /// Creates a connector without opening or starting the daemon.
    pub fn new(socket: impl Into<std::path::PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            progress: ProgressObserver::default(),
            remote_restarter: None,
        }
    }
    pub(crate) fn progress(&self) -> ProgressObserver {
        self.progress.clone()
    }

    /// Installs one startup-only observer; its retired clones ignore reconnect.
    pub fn with_progress(mut self, progress: ProgressObserver) -> Self {
        self.progress = progress;
        self
    }
    /// Injects desktop lifecycle recovery without exposing it to the protocol owner.
    pub fn with_remote_restarter(mut self, restarter: Arc<dyn RemoteDaemonRestarter>) -> Self {
        self.remote_restarter = Some(restarter);
        self
    }
}
impl AttachmentConnector for UnixAttachmentConnector {
    fn open(
        &self,
        target: ResolvedSessionTarget,
    ) -> TransportFuture<'_, Result<AttachmentTransport, DaemonError>> {
        Box::pin(async move {
            let opened = UnixAttachmentTransport::open(&self.socket, target, &self.progress).await;
            match (opened, &self.remote_restarter, target.device_id()) {
                (Err(error), Some(restarter), Some(_))
                    if error.kind() == DomainErrorKind::DaemonStopped =>
                {
                    restarter.ensure_running().await?;
                    UnixAttachmentTransport::open(&self.socket, target, &self.progress).await
                }
                (result, _, _) => result,
            }
            .map(AttachmentTransport::new)
        })
    }
}
impl AttachmentIo for UnixAttachmentTransport {
    fn queued_session_count(&self) -> usize {
        UnixAttachmentTransport::queued_session_count(self)
    }
    fn write<'a>(&'a mut self, bytes: &'a [u8]) -> TransportFuture<'a, Result<(), DaemonError>> {
        Box::pin(self.write_inner(bytes))
    }
    fn read(&mut self) -> TransportFuture<'_, Result<AttachmentTransportItem, DaemonError>> {
        Box::pin(self.read_item())
    }
    fn shutdown(&mut self) -> TransportFuture<'_, Result<(), DaemonError>> {
        Box::pin(self.shutdown_inner())
    }
}

impl zterm_client::upload::UploadConnector for UnixAttachmentConnector {
    fn open_upload(
        &self,
        target: ResolvedSessionTarget,
    ) -> TransportFuture<'_, Result<zterm_client::upload::UploadConnection, DaemonError>> {
        Box::pin(async move {
            let transport =
                UnixAttachmentTransport::open(&self.socket, target, &ProgressObserver::default())
                    .await?;
            let UnixAttachmentTransport::Tunnel {
                stream,
                remote_capabilities,
                decoder,
            } = transport
            else {
                return Err(DaemonError::new(
                    DomainErrorKind::ServiceNotImplemented,
                    "file upload requires a remote session",
                ));
            };
            let (reader, writer) = stream.into_split();
            Ok(zterm_client::upload::UploadConnection {
                capabilities: remote_capabilities,
                reader: Box::new(TunnelUploadReader { reader, decoder }),
                writer: Box::new(TunnelUploadWriter(writer)),
            })
        })
    }
}

struct TunnelUploadReader {
    reader: tokio::net::unix::OwnedReadHalf,
    decoder: TunnelDecoder,
}
impl zterm_client::upload::UploadReader for TunnelUploadReader {
    fn read(&mut self) -> TransportFuture<'_, Result<DecodedFrame, DaemonError>> {
        Box::pin(async move {
            loop {
                if let AttachmentTransportItem::Session(frame) =
                    self.decoder.read_item(&mut self.reader).await?
                {
                    return Ok(frame);
                }
            }
        })
    }
}
struct TunnelUploadWriter(tokio::net::unix::OwnedWriteHalf);
impl zterm_client::upload::UploadWriter for TunnelUploadWriter {
    fn write<'a>(&'a mut self, bytes: &'a [u8]) -> TransportFuture<'a, Result<(), DaemonError>> {
        Box::pin(write_tunnel_bytes(&mut self.0, bytes))
    }
}

pub(super) fn connect_error(error: std::io::Error) -> DaemonError {
    let kind = match error.kind() {
        std::io::ErrorKind::PermissionDenied => DomainErrorKind::PermissionMismatch,
        _ => DomainErrorKind::DaemonStopped,
    };
    DaemonError::new(kind, format!("local daemon is unavailable: {error}"))
}

pub(super) fn daemon_io(operation: &str, error: std::io::Error) -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DaemonStopped,
        format!("{operation}: {error}"),
    )
}

pub(super) fn local_attachment_command_error(error: std::io::Error) -> DaemonError {
    if is_attachment_closure_error(error.kind()) {
        zterm_client::protocol::attachment_command_stream_closed()
    } else {
        daemon_io("write local terminal message", error)
    }
}

pub(super) fn local_attachment_io(operation: &str, error: std::io::Error) -> DaemonError {
    if is_attachment_closure_error(error.kind()) {
        attachment_cancelled()
    } else {
        daemon_io(operation, error)
    }
}

pub(super) const fn is_attachment_closure_error(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::UnexpectedEof
    )
}
