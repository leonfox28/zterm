//! Socket-free global, peer, and connection stream admission gate.
//!
//! This named target owns permit overflow/isolation only. The same broker
//! module's socket-free unit tests
//! `malformed_and_oversized_first_frames_are_stream_local` and
//! `stalled_first_frame_deadline_is_stream_local` own malformed, oversized,
//! and stalled-first-frame isolation, so this target does not imply real-Iroh
//! stream coverage.

use zterm_daemon::connection_broker::stream_limit_test_evidence;

#[test]
fn overflow_is_scoped_and_every_permit_releases_by_raii() {
    let evidence = stream_limit_test_evidence();

    assert!(evidence.global_overflow_rejected);
    assert!(evidence.global_capacity_released);
    assert!(evidence.peer_overflow_rejected);
    assert!(evidence.peer_isolated);
    assert!(evidence.peer_capacity_released);
    assert!(evidence.connection_overflow_rejected);
    assert!(evidence.connection_isolated);
    assert!(evidence.connection_capacity_released);
    assert!(evidence.metric_peer_isolated);
    assert!(evidence.metric_capacity_released);
}
