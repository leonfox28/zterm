//! Same-UID opaque adapter for one authenticated remote Session service stream.
//!
//! The adapter owns only transport admission, bounded byte forwarding, and
//! address-free path observations. It deliberately does not decode the inner
//! Session stream or retain any Session identity, revision, viewport, replay,
//! or acknowledgement state.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use zterm_core::{DeviceId, DomainErrorKind};
use zterm_proto::{
    DecodedFrame, FrameDecoder, LOCAL_SESSION_TUNNEL_VERSION, MAX_LOCAL_SESSION_TUNNEL_DATA_BYTES,
    WireKind, encode_message, v2,
};

use crate::connection_broker::{
    AuthenticatedBiStream, ConnectionBroker, SelectedPathObservation, StreamPurpose,
};
use crate::error::DaemonError;
use crate::network::PathKind;
use crate::service::{ServiceReply, protocol_error};
use crate::session_wire::{FirstFrame, SessionWireLimits};

const PATH_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Pumps one local tunnel socket to one admitted service stream.
pub(crate) async fn serve_remote_session_tunnel<LocalStream>(
    broker: &ConnectionBroker,
    target: DeviceId,
    mut local_stream: LocalStream,
    first: FirstFrame,
    limits: SessionWireLimits,
    deadline: Instant,
) -> Result<(), DaemonError>
where
    LocalStream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let request_id = first.frame.request_id;
    let demand = match broker.demand(target, deadline).await {
        Ok(demand) => demand,
        Err(error) => {
            write_service_error_best_effort(&mut local_stream, request_id, &error, deadline).await;
            return Err(error);
        }
    };
    let remote = match demand.open_bi(StreamPurpose::Service, deadline).await {
        Ok(remote) => remote,
        Err(error) => {
            write_service_error_best_effort(&mut local_stream, request_id, &error, deadline).await;
            return Err(error);
        }
    };
    let observer = remote.candidate_observer();
    let result = pump_tunnel(
        local_stream,
        BrokerTunnelStream { stream: remote },
        first,
        limits,
        deadline,
        move || observer.selected_path_observation(),
    )
    .await;
    // Keep the broker demand alive for the whole admitted stream epoch.
    drop(demand);
    result
}

async fn pump_tunnel<LocalStream, RemoteStream, ObservePath>(
    local_stream: LocalStream,
    remote_stream: RemoteStream,
    first: FirstFrame,
    limits: SessionWireLimits,
    deadline: Instant,
    observe_path: ObservePath,
) -> Result<(), DaemonError>
where
    LocalStream: AsyncRead + AsyncWrite + Unpin,
    RemoteStream: AsyncRead + AsyncWrite + Unpin,
    ObservePath: Fn() -> SelectedPathObservation,
{
    let request_id = first.frame.request_id;
    let (local_reader, mut local_writer) = tokio::io::split(local_stream);
    let mut local_reader = TunnelFrameReader::from_first(local_reader, first);
    let (mut remote_reader, mut remote_writer) = tokio::io::split(remote_stream);

    write_outer(
        &mut local_writer,
        WireKind::LocalSessionTunnelOpened,
        request_id,
        &v2::LocalSessionTunnelOpened {
            protocol_version: LOCAL_SESSION_TUNNEL_VERSION,
        },
        deadline,
    )
    .await?;

    let mut previous_path = SelectedPathObservation::default();
    write_path(&mut local_writer, previous_path, deadline).await?;
    let mut path_tick = tokio::time::interval(PATH_POLL_INTERVAL);
    path_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // The first immediate interval tick would duplicate the explicit reset.
    path_tick.tick().await;

    let mut local_half_closed = false;
    let mut local_read_closed = false;
    let mut remote_buffer = vec![0_u8; MAX_LOCAL_SESSION_TUNNEL_DATA_BYTES];
    loop {
        tokio::select! {
            local = local_reader.read_frame(), if !local_read_closed => {
                let frame = match local {
                    Ok(Some(frame)) => frame,
                    Ok(None) if local_half_closed => {
                        local_read_closed = true;
                        continue;
                    }
                    Ok(None) => {
                        shutdown_remote(&mut remote_writer, limits.operation_timeout()).await;
                        return Ok(());
                    }
                    Err(error) => {
                        write_closed_best_effort(
                            &mut local_writer,
                            v2::LocalSessionTunnelCloseReason::ProtocolError,
                            limits.operation_timeout(),
                        )
                        .await;
                        return Err(error);
                    }
                };
                if local_half_closed {
                    let error = malformed("local Session tunnel sent a frame after HalfClose");
                    write_closed_best_effort(
                        &mut local_writer,
                        v2::LocalSessionTunnelCloseReason::ProtocolError,
                        limits.operation_timeout(),
                    )
                    .await;
                    return Err(error);
                }
                if let Err(error) = require_stream_frame_header(&frame) {
                    write_closed_best_effort(
                        &mut local_writer,
                        v2::LocalSessionTunnelCloseReason::ProtocolError,
                        limits.operation_timeout(),
                    )
                    .await;
                    return Err(error);
                }
                match frame.kind {
                    WireKind::LocalSessionTunnelData => {
                        let data: v2::LocalSessionTunnelData = match frame
                            .decode_message(WireKind::LocalSessionTunnelData)
                            .map_err(protocol_error)
                        {
                            Ok(data) => data,
                            Err(error) => {
                                write_closed_best_effort(
                                    &mut local_writer,
                                    v2::LocalSessionTunnelCloseReason::ProtocolError,
                                    limits.operation_timeout(),
                                )
                                .await;
                                return Err(error);
                            }
                        };
                        if let Err(error) = require_data_chunk(&data.bytes) {
                            write_closed_best_effort(
                                &mut local_writer,
                                v2::LocalSessionTunnelCloseReason::ProtocolError,
                                limits.operation_timeout(),
                            )
                            .await;
                            return Err(error);
                        }
                        if let Err(error) = write_remote(
                            &mut remote_writer,
                            &data.bytes,
                            limits.operation_timeout(),
                        )
                        .await
                        {
                            write_closed_best_effort(
                                &mut local_writer,
                                v2::LocalSessionTunnelCloseReason::TransportLost,
                                limits.operation_timeout(),
                            )
                            .await;
                            return Err(error);
                        }
                    }
                    WireKind::LocalSessionTunnelHalfClose => {
                        let decoded: Result<v2::LocalSessionTunnelHalfClose, DaemonError> = frame
                            .decode_message(WireKind::LocalSessionTunnelHalfClose)
                            .map_err(protocol_error);
                        if let Err(error) = decoded {
                            write_closed_best_effort(
                                &mut local_writer,
                                v2::LocalSessionTunnelCloseReason::ProtocolError,
                                limits.operation_timeout(),
                            )
                            .await;
                            return Err(error);
                        }
                        shutdown_remote(&mut remote_writer, limits.operation_timeout()).await;
                        local_half_closed = true;
                    }
                    _ => {
                        let error = malformed("invalid local tunnel frame after Opened");
                        write_closed_best_effort(
                            &mut local_writer,
                            v2::LocalSessionTunnelCloseReason::ProtocolError,
                            limits.operation_timeout(),
                        )
                        .await;
                        return Err(error);
                    }
                }
            }
            remote = remote_reader.read(&mut remote_buffer) => {
                match remote {
                    Ok(0) => {
                        write_outer_with_timeout(
                            &mut local_writer,
                            WireKind::LocalSessionTunnelHalfClose,
                            &v2::LocalSessionTunnelHalfClose {},
                            limits.operation_timeout(),
                        )
                        .await?;
                        write_outer_with_timeout(
                            &mut local_writer,
                            WireKind::LocalSessionTunnelClosed,
                            &v2::LocalSessionTunnelClosed {
                                reason: v2::LocalSessionTunnelCloseReason::RemoteEof as i32,
                            },
                            limits.operation_timeout(),
                        )
                        .await?;
                        let _ = timeout_after(limits.operation_timeout(), local_writer.shutdown()).await;
                        return Ok(());
                    }
                    Ok(read) => {
                        write_outer_with_timeout(
                            &mut local_writer,
                            WireKind::LocalSessionTunnelData,
                            &v2::LocalSessionTunnelData {
                                bytes: remote_buffer[..read].to_vec(),
                            },
                            limits.operation_timeout(),
                        )
                        .await?;
                    }
                    Err(_) => {
                        write_closed_best_effort(
                            &mut local_writer,
                            v2::LocalSessionTunnelCloseReason::TransportLost,
                            limits.operation_timeout(),
                        )
                        .await;
                        return Err(transport_unavailable("remote Session tunnel read failed"));
                    }
                }
            }
            _ = path_tick.tick() => {
                let observation = observe_path();
                if observation != previous_path {
                    write_path(
                        &mut local_writer,
                        observation,
                        Instant::now() + limits.operation_timeout(),
                    )
                    .await?;
                    previous_path = observation;
                }
            }
        }
    }
}

/// Decodes and validates the one Open request without exposing its target in
/// diagnostics.
pub(crate) fn decode_tunnel_open(first: &FirstFrame) -> Result<DeviceId, DaemonError> {
    if first.frame.kind != WireKind::LocalSessionTunnelOpenRequest {
        return Err(malformed("remote Session tunnel requires an Open request"));
    }
    if first.frame.request_id == 0 {
        return Err(malformed(
            "remote Session tunnel Open requires a request ID",
        ));
    }
    let request: v2::LocalSessionTunnelOpenRequest = first
        .frame
        .decode_message(WireKind::LocalSessionTunnelOpenRequest)
        .map_err(protocol_error)?;
    if request.protocol_version != LOCAL_SESSION_TUNNEL_VERSION {
        return Err(DaemonError::new(
            DomainErrorKind::WireMajorMismatch,
            "unsupported local Session tunnel protocol version",
        ));
    }
    request
        .target_device_id
        .ok_or_else(|| malformed("remote Session tunnel Open omitted target_device_id"))?
        .try_into()
        .map_err(protocol_error)
}

struct BrokerTunnelStream {
    stream: AuthenticatedBiStream,
}

impl AsyncRead for BrokerTunnelStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream.recv).poll_read(context, buffer)
    }
}

impl AsyncWrite for BrokerTunnelStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.stream.send), context, bytes)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.stream.send), context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.stream.send), context)
    }
}

struct TunnelFrameReader<Reader> {
    reader: Reader,
    decoder: FrameDecoder,
    queued: VecDeque<DecodedFrame>,
}

impl<Reader> TunnelFrameReader<Reader>
where
    Reader: AsyncRead + Unpin,
{
    fn from_first(reader: Reader, first: FirstFrame) -> Self {
        Self {
            reader,
            decoder: first.decoder,
            queued: first.queued,
        }
    }

    async fn read_frame(&mut self) -> Result<Option<DecodedFrame>, DaemonError> {
        if let Some(frame) = self.queued.pop_front() {
            return Ok(Some(frame));
        }
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = self
                .reader
                .read(&mut buffer)
                .await
                .map_err(|_| transport_unavailable("local Session tunnel read failed"))?;
            if read == 0 {
                let decoder = std::mem::replace(&mut self.decoder, FrameDecoder::new());
                decoder.finish().map_err(protocol_error)?;
                return Ok(None);
            }
            self.queued
                .extend(self.decoder.feed(&buffer[..read]).map_err(protocol_error)?);
            if let Some(frame) = self.queued.pop_front() {
                return Ok(Some(frame));
            }
        }
    }
}

fn require_stream_frame_header(frame: &DecodedFrame) -> Result<(), DaemonError> {
    if frame.request_id != 0 || frame.deadline_ms != 0 {
        Err(malformed(
            "local Session tunnel stream frames require zero request ID and deadline",
        ))
    } else {
        Ok(())
    }
}

fn require_data_chunk(bytes: &[u8]) -> Result<(), DaemonError> {
    if bytes.is_empty() {
        return Err(malformed("local Session tunnel Data must not be empty"));
    }
    if bytes.len() > MAX_LOCAL_SESSION_TUNNEL_DATA_BYTES {
        return Err(DaemonError::new(
            DomainErrorKind::ControlPayloadTooLarge,
            "local Session tunnel Data exceeds the chunk limit",
        ));
    }
    Ok(())
}

async fn write_path<Writer>(
    writer: &mut Writer,
    observation: SelectedPathObservation,
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    let path = match observation.path {
        PathKind::Unknown => v2::TerminalConnectionPath::Unknown,
        PathKind::Direct => v2::TerminalConnectionPath::Direct,
        PathKind::Relay => v2::TerminalConnectionPath::Relay,
    };
    write_outer(
        writer,
        WireKind::LocalSessionTunnelPath,
        0,
        &v2::LocalSessionTunnelPath {
            path: path as i32,
            rtt_ms: observation.rtt_ms,
        },
        deadline,
    )
    .await
}

async fn write_outer_with_timeout<Writer, Message>(
    writer: &mut Writer,
    kind: WireKind,
    message: &Message,
    timeout: Duration,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
    Message: prost::Message,
{
    write_outer(writer, kind, 0, message, Instant::now() + timeout).await
}

async fn write_outer<Writer, Message>(
    writer: &mut Writer,
    kind: WireKind,
    request_id: u64,
    message: &Message,
    deadline: Instant,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
    Message: prost::Message,
{
    let bytes = encode_message(kind, request_id, 0, message).map_err(protocol_error)?;
    timeout_at(
        deadline,
        writer.write_all(&bytes),
        "local tunnel write timed out",
    )
    .await?
    .map_err(|_| transport_unavailable("local Session tunnel write failed"))?;
    timeout_at(deadline, writer.flush(), "local tunnel flush timed out")
        .await?
        .map_err(|_| transport_unavailable("local Session tunnel flush failed"))
}

async fn write_remote<Writer>(
    writer: &mut Writer,
    bytes: &[u8],
    timeout: Duration,
) -> Result<(), DaemonError>
where
    Writer: AsyncWrite + Unpin,
{
    timeout_after(timeout, writer.write_all(bytes))
        .await?
        .map_err(|_| transport_unavailable("remote Session tunnel write failed"))?;
    timeout_after(timeout, writer.flush())
        .await?
        .map_err(|_| transport_unavailable("remote Session tunnel flush failed"))
}

async fn shutdown_remote<Writer>(writer: &mut Writer, timeout: Duration)
where
    Writer: AsyncWrite + Unpin,
{
    let _ = timeout_after(timeout, writer.shutdown()).await;
}

async fn write_closed_best_effort<Writer>(
    writer: &mut Writer,
    reason: v2::LocalSessionTunnelCloseReason,
    timeout: Duration,
) where
    Writer: AsyncWrite + Unpin,
{
    let _ = write_outer_with_timeout(
        writer,
        WireKind::LocalSessionTunnelClosed,
        &v2::LocalSessionTunnelClosed {
            reason: reason as i32,
        },
        timeout,
    )
    .await;
}

async fn write_service_error_best_effort<Writer>(
    writer: &mut Writer,
    request_id: u64,
    error: &DaemonError,
    deadline: Instant,
) where
    Writer: AsyncWrite + Unpin,
{
    let bytes = ServiceReply::error(request_id, error).bytes;
    let _ = timeout_at(
        deadline,
        writer.write_all(&bytes),
        "local tunnel error write timed out",
    )
    .await;
    let _ = timeout_at(
        deadline,
        writer.flush(),
        "local tunnel error flush timed out",
    )
    .await;
}

async fn timeout_at<F>(
    deadline: Instant,
    future: F,
    detail: &'static str,
) -> Result<F::Output, DaemonError>
where
    F: Future,
{
    if Instant::now() >= deadline {
        return Err(DaemonError::new(DomainErrorKind::DeadlineExceeded, detail));
    }
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future)
        .await
        .map_err(|_| DaemonError::new(DomainErrorKind::DeadlineExceeded, detail))
}

async fn timeout_after<F>(timeout: Duration, future: F) -> Result<F::Output, DaemonError>
where
    F: Future,
{
    tokio::time::timeout(timeout, future).await.map_err(|_| {
        DaemonError::new(
            DomainErrorKind::DeadlineExceeded,
            "tunnel operation timed out",
        )
    })
}

fn transport_unavailable(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::TransportUnavailable, detail)
}

fn malformed(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};

    fn device(byte: u8) -> DeviceId {
        DeviceId::from_array([byte; DeviceId::LENGTH])
    }

    fn envelope<Message: prost::Message>(kind: WireKind, message: &Message) -> Vec<u8> {
        encode_message(kind, 0, 0, message).expect("encode bounded tunnel test envelope")
    }

    fn first_with_queued(queued_bytes: &[u8]) -> FirstFrame {
        let mut bytes = encode_message(
            WireKind::LocalSessionTunnelOpenRequest,
            7,
            5_000,
            &v2::LocalSessionTunnelOpenRequest {
                protocol_version: LOCAL_SESSION_TUNNEL_VERSION,
                target_device_id: Some(device(0x41).into()),
            },
        )
        .expect("encode bounded tunnel Open");
        bytes.extend_from_slice(queued_bytes);
        let mut decoder = FrameDecoder::new();
        let mut queued = VecDeque::from(decoder.feed(&bytes).expect("decode tunnel test frames"));
        let frame = queued.pop_front().expect("Open is the first decoded frame");
        FirstFrame {
            frame,
            decoder,
            queued,
        }
    }

    async fn next_frame(
        stream: &mut DuplexStream,
        decoder: &mut FrameDecoder,
        queued: &mut VecDeque<DecodedFrame>,
    ) -> DecodedFrame {
        if let Some(frame) = queued.pop_front() {
            return frame;
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream
                    .read(&mut buffer)
                    .await
                    .expect("read tunnel envelope");
                assert!(read > 0, "tunnel closed before the expected envelope");
                queued.extend(
                    decoder
                        .feed(&buffer[..read])
                        .expect("decode tunnel envelope"),
                );
                if let Some(frame) = queued.pop_front() {
                    return frame;
                }
            }
        })
        .await
        .expect("tunnel envelope arrived before the test deadline")
    }

    async fn expect_opened_and_reset(
        stream: &mut DuplexStream,
        decoder: &mut FrameDecoder,
        queued: &mut VecDeque<DecodedFrame>,
    ) {
        let opened = next_frame(stream, decoder, queued).await;
        assert_eq!(opened.kind, WireKind::LocalSessionTunnelOpened);
        assert_eq!(opened.request_id, 7);
        let path = next_frame(stream, decoder, queued).await;
        assert_eq!(path.kind, WireKind::LocalSessionTunnelPath);
        let path: v2::LocalSessionTunnelPath = path
            .decode_message(WireKind::LocalSessionTunnelPath)
            .expect("decode initial path reset");
        assert_eq!(
            v2::TerminalConnectionPath::try_from(path.path),
            Ok(v2::TerminalConnectionPath::Unknown)
        );
        assert_eq!(path.rtt_ms, None);
    }

    #[tokio::test]
    async fn opaque_pump_forwards_queued_data_and_propagates_half_close_once() {
        let first_chunk = envelope(
            WireKind::LocalSessionTunnelData,
            &v2::LocalSessionTunnelData {
                bytes: b"first".to_vec(),
            },
        );
        let first = first_with_queued(&first_chunk);
        let (local_daemon, mut frontend) = duplex(1024 * 1024);
        let (remote_daemon, mut target) = duplex(1024 * 1024);
        let deadline = Instant::now() + Duration::from_secs(3);
        let task = tokio::spawn(pump_tunnel(
            local_daemon,
            remote_daemon,
            first,
            SessionWireLimits::default(),
            deadline,
            SelectedPathObservation::default,
        ));

        let mut decoder = FrameDecoder::new();
        let mut queued = VecDeque::new();
        expect_opened_and_reset(&mut frontend, &mut decoder, &mut queued).await;
        let mut forwarded = [0_u8; 5];
        target
            .read_exact(&mut forwarded)
            .await
            .expect("queued bytes reach the target");
        assert_eq!(&forwarded, b"first");

        frontend
            .write_all(&envelope(
                WireKind::LocalSessionTunnelData,
                &v2::LocalSessionTunnelData {
                    bytes: b"second".to_vec(),
                },
            ))
            .await
            .expect("write another bounded frontend chunk");
        let mut forwarded = [0_u8; 6];
        target
            .read_exact(&mut forwarded)
            .await
            .expect("later bytes reach the target");
        assert_eq!(&forwarded, b"second");

        target
            .write_all(b"reply")
            .await
            .expect("target writes opaque response bytes");
        let response = next_frame(&mut frontend, &mut decoder, &mut queued).await;
        assert_eq!(response.kind, WireKind::LocalSessionTunnelData);
        let response: v2::LocalSessionTunnelData = response
            .decode_message(WireKind::LocalSessionTunnelData)
            .expect("decode opaque response envelope");
        assert_eq!(response.bytes, b"reply");

        frontend
            .write_all(&envelope(
                WireKind::LocalSessionTunnelHalfClose,
                &v2::LocalSessionTunnelHalfClose {},
            ))
            .await
            .expect("frontend half-closes its tunnel direction");
        let mut eof_probe = [0_u8; 1];
        assert_eq!(
            target.read(&mut eof_probe).await.expect("read target EOF"),
            0
        );
        target
            .shutdown()
            .await
            .expect("target half-closes response");

        let half_close = next_frame(&mut frontend, &mut decoder, &mut queued).await;
        assert_eq!(half_close.kind, WireKind::LocalSessionTunnelHalfClose);
        let closed = next_frame(&mut frontend, &mut decoder, &mut queued).await;
        assert_eq!(closed.kind, WireKind::LocalSessionTunnelClosed);
        let closed: v2::LocalSessionTunnelClosed = closed
            .decode_message(WireKind::LocalSessionTunnelClosed)
            .expect("decode terminal close envelope");
        assert_eq!(
            v2::LocalSessionTunnelCloseReason::try_from(closed.reason),
            Ok(v2::LocalSessionTunnelCloseReason::RemoteEof)
        );
        task.await
            .expect("tunnel pump task joins")
            .expect("clean half-close completes the pump");
    }

    #[tokio::test]
    async fn frontend_data_after_half_close_is_a_tunnel_local_protocol_error() {
        let mut queued = envelope(
            WireKind::LocalSessionTunnelHalfClose,
            &v2::LocalSessionTunnelHalfClose {},
        );
        queued.extend_from_slice(&envelope(
            WireKind::LocalSessionTunnelData,
            &v2::LocalSessionTunnelData {
                bytes: b"must-not-forward".to_vec(),
            },
        ));
        let (local_daemon, mut frontend) = duplex(1024 * 1024);
        let (remote_daemon, mut target) = duplex(1024 * 1024);
        let deadline = Instant::now() + Duration::from_secs(3);
        let task = tokio::spawn(pump_tunnel(
            local_daemon,
            remote_daemon,
            first_with_queued(&queued),
            SessionWireLimits::default(),
            deadline,
            SelectedPathObservation::default,
        ));

        let mut decoder = FrameDecoder::new();
        let mut frames = VecDeque::new();
        expect_opened_and_reset(&mut frontend, &mut decoder, &mut frames).await;
        let closed = next_frame(&mut frontend, &mut decoder, &mut frames).await;
        assert_eq!(closed.kind, WireKind::LocalSessionTunnelClosed);
        let closed: v2::LocalSessionTunnelClosed = closed
            .decode_message(WireKind::LocalSessionTunnelClosed)
            .expect("decode post-HalfClose protocol failure");
        assert_eq!(
            v2::LocalSessionTunnelCloseReason::try_from(closed.reason),
            Ok(v2::LocalSessionTunnelCloseReason::ProtocolError)
        );
        let mut probe = [0_u8; 1];
        assert_eq!(
            target
                .read(&mut probe)
                .await
                .expect("target sees the propagated write half-close"),
            0,
            "Data after HalfClose must never reach the target Session stream"
        );
        let error = task
            .await
            .expect("post-HalfClose tunnel task joins")
            .expect_err("post-HalfClose Data is rejected");
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
    }

    #[tokio::test]
    async fn malformed_tunnel_closes_only_its_own_stream() {
        let bad_data = envelope(
            WireKind::LocalSessionTunnelData,
            &v2::LocalSessionTunnelData { bytes: Vec::new() },
        );
        let good_data = envelope(
            WireKind::LocalSessionTunnelData,
            &v2::LocalSessionTunnelData {
                bytes: b"healthy".to_vec(),
            },
        );
        let (bad_local, mut bad_frontend) = duplex(1024 * 1024);
        let (bad_remote, _bad_target) = duplex(1024 * 1024);
        let (good_local, mut good_frontend) = duplex(1024 * 1024);
        let (good_remote, mut good_target) = duplex(1024 * 1024);
        let deadline = Instant::now() + Duration::from_secs(3);
        let bad_task = tokio::spawn(pump_tunnel(
            bad_local,
            bad_remote,
            first_with_queued(&bad_data),
            SessionWireLimits::default(),
            deadline,
            SelectedPathObservation::default,
        ));
        let good_task = tokio::spawn(pump_tunnel(
            good_local,
            good_remote,
            first_with_queued(&good_data),
            SessionWireLimits::default(),
            deadline,
            SelectedPathObservation::default,
        ));

        let mut bad_decoder = FrameDecoder::new();
        let mut bad_queued = VecDeque::new();
        let mut good_decoder = FrameDecoder::new();
        let mut good_queued = VecDeque::new();
        expect_opened_and_reset(&mut bad_frontend, &mut bad_decoder, &mut bad_queued).await;
        expect_opened_and_reset(&mut good_frontend, &mut good_decoder, &mut good_queued).await;
        let closed = next_frame(&mut bad_frontend, &mut bad_decoder, &mut bad_queued).await;
        assert_eq!(closed.kind, WireKind::LocalSessionTunnelClosed);
        let closed: v2::LocalSessionTunnelClosed = closed
            .decode_message(WireKind::LocalSessionTunnelClosed)
            .expect("decode malformed-stream close");
        assert_eq!(
            v2::LocalSessionTunnelCloseReason::try_from(closed.reason),
            Ok(v2::LocalSessionTunnelCloseReason::ProtocolError)
        );
        let error = bad_task
            .await
            .expect("malformed tunnel task joins")
            .expect_err("zero-byte Data is rejected");
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);

        let mut healthy = [0_u8; 7];
        good_target
            .read_exact(&mut healthy)
            .await
            .expect("sibling tunnel remains independently live");
        assert_eq!(&healthy, b"healthy");
        good_task.abort();
        assert!(
            good_task
                .await
                .expect_err("explicit fixture cancellation stops only the sibling task")
                .is_cancelled()
        );
    }
}
