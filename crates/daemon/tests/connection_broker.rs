//! Real-Iroh singleflight and authenticated stream integration evidence.

use std::time::{Duration, Instant};

use iroh::SecretKey;
use zterm_core::{DeviceId, DomainErrorKind, TransportLimits};
use zterm_daemon::connection_broker::StreamPurpose;
use zterm_proto::{FrameDecoder, WireKind, encode_message, v1};

#[path = "support/network_fixture.rs"]
mod network_fixture;
use network_fixture::NetworkPeer;

fn device(secret: [u8; 32]) -> DeviceId {
    DeviceId::from_array(*SecretKey::from_bytes(&secret).public().as_bytes())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(
    target_os = "macos",
    ignore = "real Iroh UDP listeners are disabled on macOS to avoid application firewall prompts"
)]
async fn concurrent_demands_and_streams_share_one_primary_dial() {
    let a_id = device([0x11; 32]);
    let b_id = device([0x22; 32]);
    let limits = TransportLimits::default();
    let a = NetworkPeer::create([0x11; 32], "host-a", &[(b_id, "host-b")], &[], limits).await;
    let b = NetworkPeer::create(
        [0x22; 32],
        "host-b",
        &[],
        &[(a_id, "host-a", "host-a")],
        limits,
    )
    .await;
    b.broker
        .set_test_route(a_id, a.address())
        .expect("direct fixture route");

    let mut tasks = tokio::task::JoinSet::new();
    for request_id in 1..=8_u64 {
        let broker = b.broker.clone();
        tasks.spawn(async move {
            let deadline = Instant::now() + Duration::from_secs(10);
            let demand = broker
                .demand(a_id, deadline)
                .await
                .map_err(|error| error.to_string())?;
            let mut stream = demand
                .open_bi(StreamPurpose::Service, deadline)
                .await
                .map_err(|error| error.to_string())?;
            let request = encode_message(
                WireKind::SessionListRequest,
                request_id,
                0,
                &v1::SessionListRequest { target: None },
            )
            .map_err(|error| error.to_string())?;
            stream
                .send
                .write_all(&request)
                .await
                .map_err(|_| "request write failed".to_owned())?;
            stream
                .send
                .finish()
                .map_err(|_| "request finish failed".to_owned())?;
            let frame =
                tokio::time::timeout(Duration::from_secs(5), read_response(&mut stream.recv))
                    .await
                    .map_err(|_| "response timeout".to_owned())??;
            if frame.kind != WireKind::ServiceErrorResponse || frame.request_id != request_id {
                return Err("unexpected service response".to_owned());
            }
            let error: v1::ServiceError = frame
                .decode_message(WireKind::ServiceErrorResponse)
                .map_err(|error| error.to_string())?;
            if error.code != DomainErrorKind::ServiceNotImplemented.code() {
                return Err(format!("unexpected error code {}", error.code));
            }
            Ok::<_, String>(())
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.expect("stream task joins").expect("stream succeeds");
    }

    a.wait_for_primary(b_id).await;
    b.wait_for_primary(a_id).await;
    let a_observation = a.broker.peer_observation(b_id).await;
    let b_observation = b.broker.peer_observation(a_id).await;
    assert_eq!(a_observation.primary, b_observation.primary);
    assert_eq!(a_observation.candidate_count, 1);
    assert_eq!(b_observation.candidate_count, 1);
    assert_eq!(b.broker.observe().snapshot().primary_connection_count, 1);

    b.shutdown().await;
    a.shutdown().await;
}

async fn read_response(
    recv: &mut iroh::endpoint::RecvStream,
) -> Result<zterm_proto::DecodedFrame, String> {
    let mut decoder = FrameDecoder::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = recv
            .read(&mut buffer)
            .await
            .map_err(|_| "response read failed".to_owned())?;
        let Some(read) = read else {
            return Err("response ended before a frame".to_owned());
        };
        let mut frames = decoder
            .feed(&buffer[..read])
            .map_err(|error| error.to_string())?;
        if frames.len() == 1 {
            return Ok(frames.remove(0));
        }
        if frames.len() > 1 {
            return Err("multiple response frames".to_owned());
        }
    }
}
