//! Inbound authorization registry acceptance tests.

#[path = "support/state_fixture.rs"]
mod state_fixture;

use tokio::sync::{mpsc, oneshot};
use zterm_core::{
    AuthGeneration, AuthorizationSnapshot, AuthorizationStatus, DeviceDisplayName, DeviceId,
    DomainErrorKind,
};
use zterm_daemon::authorization::AuthorizationRegistry;
use zterm_daemon::error::DaemonError;
use zterm_daemon::store::{DeviceAuthorization, StateStore, StoreActor, default_store_deadline};

use state_fixture::TestState;

fn generation(value: u64) -> AuthGeneration {
    AuthGeneration::new(value).expect("generation within the SQLite signed ceiling")
}

fn authorization(
    device_id: DeviceId,
    status: AuthorizationStatus,
    value: u64,
) -> DeviceAuthorization {
    DeviceAuthorization {
        device_id,
        display_name: DeviceDisplayName::new("peer").expect("display name"),
        status,
        generation: generation(value),
        paired_at_unix: 1,
        revoked_at_unix: None,
        last_seen_at_unix: None,
    }
}

#[test]
fn preload_snapshot_and_admission_reflect_authorization() {
    let device = DeviceId::from_array([0x11; 32]);
    let registry = AuthorizationRegistry::new();

    // A device with no row is neither authorized nor admitted.
    assert_eq!(
        registry.snapshot(device).expect("snapshot"),
        AuthorizationSnapshot::none()
    );
    assert_eq!(
        registry
            .admit(device)
            .expect_err("unauthorized admit")
            .kind(),
        DomainErrorKind::Unauthorized
    );

    registry
        .preload(vec![authorization(
            device,
            AuthorizationStatus::Authorized,
            3,
        )])
        .expect("preload");
    assert_eq!(
        registry.snapshot(device).expect("snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: generation(3),
        }
    );
    let admission = registry.admit(device).expect("admit");
    assert_eq!(
        admission.snapshot,
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: generation(3),
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoked_and_unknown_devices_are_rejected() {
    let revoked = DeviceId::from_array([0x22; 32]);
    let unknown = DeviceId::from_array([0x23; 32]);
    let registry = AuthorizationRegistry::new();
    registry
        .preload(vec![authorization(
            revoked,
            AuthorizationStatus::Revoked,
            5,
        )])
        .expect("preload");

    assert_eq!(
        registry.admit(revoked).expect_err("revoked admit").kind(),
        DomainErrorKind::Unauthorized
    );
    assert_eq!(
        registry.admit(unknown).expect_err("unknown admit").kind(),
        DomainErrorKind::Unauthorized
    );
    assert_eq!(
        registry
            .revoke_guard(unknown)
            .await
            .expect_err("revoke unknown")
            .kind(),
        DomainErrorKind::DeviceNotFound
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn commit_runs_closure_and_generation_mismatch_is_rejected() {
    let device = DeviceId::from_array([0x31; 32]);
    let registry = AuthorizationRegistry::new();
    registry
        .preload(vec![authorization(
            device,
            AuthorizationStatus::Authorized,
            1,
        )])
        .expect("preload");

    // A commit under the current generation runs its side effect.
    let context = registry
        .acquire_commit(device, generation(1))
        .await
        .expect("current generation");
    assert_eq!(
        context
            .run(|| Ok::<u32, DaemonError>(42))
            .await
            .expect("side effect"),
        42
    );

    // A wrong generation is rejected before any side effect.
    assert_eq!(
        registry
            .acquire_commit(device, generation(2))
            .await
            .expect_err("future generation")
            .kind(),
        DomainErrorKind::AuthorizationRevoked
    );

    // Re-authorizing advances the generation while remaining authorized.
    {
        let mut guard = registry
            .authorize_guard(device)
            .await
            .expect("authorize guard");
        guard
            .publish(AuthorizationSnapshot {
                status: AuthorizationStatus::Authorized,
                generation: generation(2),
            })
            .expect("publish reauthorization");
    }
    assert_eq!(
        registry
            .acquire_commit(device, generation(1))
            .await
            .expect_err("stale generation")
            .kind(),
        DomainErrorKind::AuthorizationRevoked
    );
    registry
        .acquire_commit(device, generation(2))
        .await
        .expect("advanced generation");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_waits_for_in_flight_commit_and_rejects_stale_generation() {
    let device = DeviceId::from_array([0x32; 32]);
    let registry = AuthorizationRegistry::new();
    registry
        .preload(vec![authorization(
            device,
            AuthorizationStatus::Authorized,
            1,
        )])
        .expect("preload");

    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let committer_registry = registry.clone();
    let committer = tokio::spawn(async move {
        let context = committer_registry
            .acquire_commit(device, generation(1))
            .await
            .expect("acquire commit");
        entered_tx.send(()).expect("signal entered");
        let _ = release_rx.await;
        context
            .run(|| Ok::<(), DaemonError>(()))
            .await
            .expect("commit completes");
    });

    // Wait until the committer actually holds the read permit.
    entered_rx.await.expect("committer holds read permit");

    let revoker_registry = registry.clone();
    let (writer_started_tx, writer_started_rx) = oneshot::channel();
    let (writer_acquired_tx, writer_acquired_rx) = oneshot::channel();
    let (publish_tx, publish_rx) = oneshot::channel();
    let revoker = tokio::spawn(async move {
        // This send does not yield. The task next polls the write lock and can
        // yield only after Tokio has queued the writer behind the held reader.
        writer_started_tx.send(()).expect("signal writer start");
        let mut guard = revoker_registry
            .revoke_guard(device)
            .await
            .expect("write permit");
        writer_acquired_tx.send(()).expect("signal writer acquired");
        publish_rx.await.expect("release writer publish");
        guard
            .publish(AuthorizationSnapshot {
                status: AuthorizationStatus::Revoked,
                generation: generation(2),
            })
            .expect("publish revoke");
    });
    writer_started_rx.await.expect("writer is queued");

    // A reader arriving after the queued writer must not overtake it. Like the
    // writer barrier above, the start signal is immediately followed by the
    // first poll of the lock future before this task can yield.
    let later_registry = registry.clone();
    let (reader_started_tx, reader_started_rx) = oneshot::channel();
    let (reader_result_tx, mut reader_result_rx) = mpsc::channel(1);
    let later_reader = tokio::spawn(async move {
        reader_started_tx.send(()).expect("signal reader start");
        let result = later_registry
            .acquire_commit(device, generation(1))
            .await
            .map(|_| ())
            .map_err(|error| error.kind());
        reader_result_tx
            .send(result)
            .await
            .expect("send reader result");
    });
    reader_started_rx.await.expect("later reader is queued");

    // Neither queued waiter can finish while the original commit holds its
    // permit.
    assert!(
        reader_result_rx.try_recv().is_err(),
        "later reader completed while the original commit held its permit"
    );

    release_tx.send(()).expect("release committer");
    committer.await.expect("committer joins");

    // Fairness gives the already-queued writer the lock first. Hold it at a
    // deterministic barrier and prove the later reader still cannot acquire.
    writer_acquired_rx.await.expect("writer acquires first");
    assert!(
        reader_result_rx.try_recv().is_err(),
        "later reader overtook the queued writer"
    );
    publish_tx.send(()).expect("allow revoke publication");
    revoker.await.expect("revoker joins");
    assert_eq!(
        reader_result_rx.recv().await.expect("reader completes"),
        Err(DomainErrorKind::Unauthorized)
    );
    later_reader.await.expect("later reader joins");

    assert_eq!(
        registry.snapshot(device).expect("snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Revoked,
            generation: generation(2),
        }
    );
    assert_eq!(
        registry
            .acquire_commit(device, generation(1))
            .await
            .expect_err("stale after revoke")
            .kind(),
        DomainErrorKind::Unauthorized
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_store_revoke_does_not_publish_in_memory() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let mut store = StateStore::open(&state.paths).expect("store opens");
    let device = DeviceId::from_array([0x41; 32]);
    store
        .authorize_device(device, "peer", 1)
        .expect("authorize");

    // Inject a real SQLite abort at the mutation boundary. This exercises
    // transaction rollback rather than the separate generation preflight.
    {
        let connection =
            rusqlite::Connection::open(state.paths.database()).expect("fixture connection");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_revoke
                 BEFORE UPDATE OF status ON device_auth
                 WHEN NEW.status=2
                 BEGIN SELECT RAISE(ABORT, 'injected revoke abort'); END;",
            )
            .expect("install injected abort trigger");
    }

    let actor = StoreActor::start(store).expect("actor starts");
    let handle = actor.handle();
    let rows = handle
        .list_authorizations(default_store_deadline())
        .expect("list authorizations");
    let registry = AuthorizationRegistry::new();
    registry.preload(rows).expect("preload");

    let initial = AuthorizationSnapshot {
        status: AuthorizationStatus::Authorized,
        generation: generation(1),
    };
    assert_eq!(registry.snapshot(device).expect("snapshot"), initial);

    // The revoke coordinator holds the write permit across the durable write;
    // when that write fails, it must not publish.
    {
        let guard = registry.revoke_guard(device).await.expect("write permit");
        assert_eq!(guard.snapshot(), initial);
        let deadline = default_store_deadline();
        let durable = handle
            .run_blocking_until(deadline, move |store, deadline| {
                store.revoke(device, 2, deadline)
            })
            .await
            .expect_err("injected SQLite abort");
        assert_eq!(durable.kind(), DomainErrorKind::StoreUnavailable);
        // No publish; the guard is dropped with the durable and in-memory
        // state unchanged.
    }

    assert_eq!(registry.snapshot(device).expect("in-memory"), initial);
    assert_eq!(
        handle
            .authorization_snapshot(device, default_store_deadline())
            .expect("durable"),
        initial
    );

    // Remove the injected failure and retry the same ordered coordinator
    // sequence. The durable write succeeds first, then publication advances
    // the in-memory snapshot to the exact committed generation.
    {
        let connection =
            rusqlite::Connection::open(state.paths.database()).expect("fixture connection");
        connection
            .execute_batch("DROP TRIGGER fail_revoke;")
            .expect("remove injected abort trigger");
    }
    let mut guard = registry
        .revoke_guard(device)
        .await
        .expect("retry write permit");
    let deadline = default_store_deadline();
    let committed = handle
        .run_blocking_until(deadline, move |store, deadline| {
            store.revoke(device, 3, deadline)
        })
        .await
        .expect("retry durable revoke");
    assert_eq!(committed, generation(2));
    let revoked = AuthorizationSnapshot {
        status: AuthorizationStatus::Revoked,
        generation: committed,
    };
    guard.publish(revoked).expect("publish committed revoke");
    drop(guard);
    assert_eq!(registry.snapshot(device).expect("in-memory"), revoked);
    assert_eq!(
        handle
            .authorization_snapshot(device, default_store_deadline())
            .expect("durable"),
        revoked
    );
    actor.shutdown();
}
