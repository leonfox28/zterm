//! Socket-free duplicate-candidate arbitration gate.

use std::sync::{Arc, Barrier};
use std::thread;

use zterm_core::{ConnectionAttemptId, ConnectionCandidateKey, DeviceId};
use zterm_daemon::connection_broker::{
    DuplicateConnectionTestEvidence, duplicate_connection_test_evidence,
};

fn key(initiator: u8, attempt: u8) -> ConnectionCandidateKey {
    ConnectionCandidateKey::new(
        DeviceId::from_array([initiator; 32]),
        ConnectionAttemptId::from_array([attempt; 16]),
    )
}

#[test]
fn barrier_schedules_converge_and_loser_cleanup_leaves_no_ghost() -> Result<(), String> {
    let lower = key(0x11, 1);
    let higher = key(0x22, 1);
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for higher_arrives_first in [true, false] {
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            barrier.wait();
            duplicate_connection_test_evidence(lower, higher, higher_arrives_first)
                .map_err(|error| error.to_string())
        }));
    }

    barrier.wait();
    let mut evidence = Vec::new();
    for worker in workers {
        evidence.push(
            worker
                .join()
                .map_err(|_| "duplicate schedule worker panicked".to_owned())??,
        );
    }

    let expected = DuplicateConnectionTestEvidence {
        primary: Some(lower),
        remaining_candidate_count: 1,
        loser_count: 1,
        redial_suppressed_while_provisional: true,
        redial_suppressed_after_confirmation: true,
        empty_after_peer_close: true,
    };
    assert_eq!(evidence, vec![expected.clone(), expected]);
    Ok(())
}
