// Baseline-only observer for fde63f6; post-fix assertions live in client::session tests.
//! Isolated fault observations against unchanged public attachment APIs.
//! Run through run_probe.py; no real daemon, identity, network, or PTY is used.

use std::collections::VecDeque;
use std::error::Error;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use zterm_core::terminal::TerminalSize;
use zterm_core::{AttachmentId, SessionId};
use zterm_daemon::local_ipc::LocalAttachmentClient;
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v2};

type ProbeError = Box<dyn Error + Send + Sync>;
const OBSERVATION_BOUND: Duration = Duration::from_secs(6);
const DEFERRED_EVENTS: usize = 128;

async fn next_frame(
    stream: &mut UnixStream,
    decoder: &mut FrameDecoder,
    queued: &mut VecDeque<DecodedFrame>,
) -> Result<DecodedFrame, ProbeError> {
    loop {
        if let Some(frame) = queued.pop_front() {
            return Ok(frame);
        }
        let mut bytes = [0; 4096];
        let count = stream.read(&mut bytes).await?;
        if count == 0 {
            return Err("fixture stream ended early".into());
        }
        queued.extend(decoder.feed(&bytes[..count])?);
    }
}

async fn accept_attachment(
    listener: UnixListener,
) -> Result<
    (
        UnixStream,
        FrameDecoder,
        VecDeque<DecodedFrame>,
        AttachmentId,
    ),
    ProbeError,
> {
    let (mut stream, _) = listener.accept().await?;
    let mut decoder = FrameDecoder::new();
    let mut queued = VecDeque::new();
    let attach = next_frame(&mut stream, &mut decoder, &mut queued).await?;
    assert_eq!(attach.kind, WireKind::TerminalAttachRequest);
    let attachment_id = AttachmentId::from_array([2; 16]);
    let model = zterm_terminal::TerminalModel::new(TerminalSize::new(2, 8), 0)?;
    let message = zterm_proto::terminal_surface_snapshot_message(
        SessionId::from_array([1; 16]),
        attachment_id,
        model.snapshot(),
    );
    stream
        .write_all(&encode_message(
            WireKind::TerminalSemanticSnapshot,
            attach.request_id,
            0,
            &message,
        )?)
        .await?;
    Ok((stream, decoder, queued, attachment_id))
}

async fn observe_missing_lease_response() -> Result<(), ProbeError> {
    let temporary = tempfile::tempdir()?;
    let socket = temporary.path().join("lease.sock");
    let listener = UnixListener::bind(&socket)?;
    let (release, hold) = tokio::sync::oneshot::channel::<()>();
    let (sent, observed) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (mut stream, mut decoder, mut queued, attachment_id) =
            accept_attachment(listener).await?;
        loop {
            let frame = next_frame(&mut stream, &mut decoder, &mut queued).await?;
            if frame.kind == WireKind::SessionOperationLeaseRequest {
                break;
            }
        }
        let event = encode_message(
            WireKind::TerminalTransportStateEvent,
            0,
            0,
            &v2::TerminalTransportStateEvent {
                attachment_id: Some(attachment_id.into()),
                state: v2::TerminalTransportState::Synchronizing as i32,
            },
        )?;
        for _ in 0..DEFERRED_EVENTS {
            stream.write_all(&event).await?;
        }
        let _ = sent.send(());
        let _ = hold.await;
        Ok::<_, ProbeError>(())
    });
    let mut client = LocalAttachmentClient::connect_main(&socket, None).await?;
    client
        .snapshot_applied(client.initial_snapshot().revision)
        .await?;
    let outcome = tokio::time::timeout(OBSERVATION_BOUND, client.begin_takeover()).await;
    observed.await?;
    let state = format!("{client:?}");
    assert!(
        outcome.is_err(),
        "baseline unexpectedly bounded the lease wait"
    );
    assert!(state.contains(&format!("deferred_frames: {DEFERRED_EVENTS}")));
    println!("LEASE_WAIT: still pending at 6 s; {DEFERRED_EVENTS} unrelated frames retained");
    drop(client);
    let _ = release.send(());
    peer.await??;
    Ok(())
}

async fn observe_blocked_write() -> Result<(), ProbeError> {
    let temporary = tempfile::tempdir()?;
    let socket = temporary.path().join("write.sock");
    let listener = UnixListener::bind(&socket)?;
    let (release, hold) = tokio::sync::oneshot::channel::<()>();
    let peer = tokio::spawn(async move {
        let (stream, _, _, _) = accept_attachment(listener).await?;
        let _ = hold.await;
        drop(stream);
        Ok::<_, ProbeError>(())
    });
    let mut client = LocalAttachmentClient::connect_main(&socket, None).await?;
    let mut completed = 0;
    let outcome = tokio::time::timeout(OBSERVATION_BOUND, async {
        for _ in 0..32 {
            client.write_input(vec![b'x'; 900_000]).await?;
            completed += 1;
        }
        Ok::<_, zterm_daemon::error::DaemonError>(())
    })
    .await;
    assert!(outcome.is_err(), "fixture did not reach a blocked write");
    println!("CONTROL_WRITE: still pending at 6 s; {completed} full input messages completed");
    drop(client);
    let _ = release.send(());
    peer.await??;
    Ok(())
}

fn main() -> Result<(), ProbeError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            observe_missing_lease_response().await?;
            observe_blocked_write().await
        })
}
