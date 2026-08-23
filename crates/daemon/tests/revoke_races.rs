//! Deterministic authorization-revoke ordering and rollback gate.
//!
//! This target uses only a task-private StoreActor/SQLite database, an
//! in-memory AuthorizationRegistry, a fake RemoteDeviceAccess, SessionService,
//! and a same-UID Unix socket. It never creates an Iroh Endpoint, binds UDP,
//! performs DNS, or contacts public infrastructure.

#![cfg(unix)]

#[path = "support/session_fixture.rs"]
mod session_fixture;
#[path = "support/state_fixture.rs"]
mod state_fixture;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, mpsc, oneshot};
use zterm_core::{
    AttachmentPrincipal, AuthGeneration, AuthorizationSnapshot, AuthorizationStatus, DeviceAlias,
    DeviceId, DomainErrorKind, RelayHint, ResourceLimits, SessionSelector,
};
use zterm_daemon::authorization::AuthorizationRegistry;
use zterm_daemon::bootstrap::bootstrap;
use zterm_daemon::config::{ValidatedInfrastructure, validate_setup_input};
use zterm_daemon::device_directory::DeviceDirectory;
use zterm_daemon::error::DaemonError;
use zterm_daemon::local_ipc::{
    LocalClient, LocalDeviceClient, LocalIpcLimits, serve_local_with_limits,
};
use zterm_daemon::service::{
    DaemonService, DeviceLiveObservation, DeviceManagement, RemoteDeviceAccess,
};
use zterm_daemon::session::SessionService;
use zterm_daemon::store::{
    RelayRouteCache, StateStore, StoreActor, StoreHandle, default_store_deadline,
};
use zterm_platform::local_unix::{DaemonLock, bind_daemon_socket, remove_own_socket};

use session_fixture::Fixture as SessionFixture;
use state_fixture::TestState;

const REMOTE: DeviceId = DeviceId::from_array([0x81; 32]);
const OTHER: DeviceId = DeviceId::from_array([0x82; 32]);

fn generation(value: u64) -> AuthGeneration {
    AuthGeneration::new(value).expect("valid authorization generation")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CloseEvidence {
    device_id: DeviceId,
    durable: AuthorizationSnapshot,
    memory: AuthorizationSnapshot,
}

struct BarrierRemoteAccess {
    store: StoreHandle,
    registry: AuthorizationRegistry,
    live: Mutex<BTreeMap<DeviceId, DeviceLiveObservation>>,
    evidence: Mutex<Vec<CloseEvidence>>,
    close_calls: AtomicUsize,
    close_entered: Notify,
    release_first_close: Notify,
    order: Arc<Mutex<Vec<&'static str>>>,
}

impl BarrierRemoteAccess {
    fn new(
        store: StoreHandle,
        registry: AuthorizationRegistry,
        order: Arc<Mutex<Vec<&'static str>>>,
    ) -> Self {
        Self {
            store,
            registry,
            live: Mutex::new(BTreeMap::new()),
            evidence: Mutex::new(Vec::new()),
            close_calls: AtomicUsize::new(0),
            close_entered: Notify::new(),
            release_first_close: Notify::new(),
            order,
        }
    }

    fn set_live(&self, device_id: DeviceId, observation: DeviceLiveObservation) {
        lock(&self.live).insert(device_id, observation);
    }

    fn evidence(&self) -> Vec<CloseEvidence> {
        lock(&self.evidence).clone()
    }

    async fn wait_for_close_count(&self, expected: usize) -> CloseEvidence {
        loop {
            let notified = self.close_entered.notified();
            if let Some(evidence) = lock(&self.evidence).get(expected - 1).copied() {
                return evidence;
            }
            notified.await;
        }
    }

    fn release_first_close(&self) {
        self.release_first_close.notify_one();
    }
}

impl RemoteDeviceAccess for BarrierRemoteAccess {
    fn observe<'a>(
        &'a self,
        device_id: DeviceId,
        _deadline: Instant,
    ) -> Pin<Box<dyn Future<Output = Result<DeviceLiveObservation, DaemonError>> + Send + 'a>> {
        Box::pin(async move {
            Ok(lock(&self.live)
                .get(&device_id)
                .copied()
                .unwrap_or_default())
        })
    }

    fn close_remote<'a>(
        &'a self,
        device_id: DeviceId,
        deadline: Instant,
    ) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'a>> {
        Box::pin(async move {
            let evidence = CloseEvidence {
                device_id,
                durable: self.store.authorization_snapshot(device_id, deadline)?,
                memory: self.registry.snapshot(device_id)?,
            };
            lock(&self.order).push("close");
            lock(&self.evidence).push(evidence);
            self.close_entered.notify_waiters();

            if self.close_calls.fetch_add(1, Ordering::AcqRel) == 0 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                tokio::time::timeout(remaining, self.release_first_close.notified())
                    .await
                    .map_err(|_| {
                        DaemonError::new(
                            DomainErrorKind::DeadlineExceeded,
                            "test remote-close barrier exceeded the revoke deadline",
                        )
                    })?;
            }
            lock(&self.live).remove(&device_id);
            Ok(())
        })
    }
}

struct Harness {
    state: TestState,
    lock: DaemonLock,
    actor: StoreActor,
    store: StoreHandle,
    server: tokio::task::JoinHandle<Result<(), DaemonError>>,
    access: Arc<BarrierRemoteAccess>,
    registry: AuthorizationRegistry,
    sessions: SessionService,
    before_revoke: mpsc::UnboundedReceiver<DeviceId>,
    order: Arc<Mutex<Vec<&'static str>>>,
}

impl Harness {
    fn start(sessions: SessionService, populate: impl FnOnce(&mut StateStore)) -> Self {
        let state = TestState::new();
        let requested =
            validate_setup_input("revoke-race-host", ValidatedInfrastructure::OfficialN0)
                .expect("valid setup");
        let setup = bootstrap(&state.paths, &requested).expect("bootstrap");
        let mut store = StateStore::open(&state.paths).expect("state store");
        populate(&mut store);
        let authorizations = store.list_authorizations().expect("authorization preload");
        let actor = StoreActor::start(store).expect("store actor starts");
        let handle = actor.handle();
        let registry = AuthorizationRegistry::new();
        registry
            .preload(authorizations)
            .expect("authorization registry preload");
        let directory = DeviceDirectory::new(handle.clone());
        let order = Arc::new(Mutex::new(Vec::new()));
        let access = Arc::new(BarrierRemoteAccess::new(
            handle.clone(),
            registry.clone(),
            Arc::clone(&order),
        ));
        let remote_access: Arc<dyn RemoteDeviceAccess> = access.clone();
        let (before_revoke_tx, before_revoke) = mpsc::unbounded_channel();
        let management =
            DeviceManagement::new(handle.clone(), directory, registry.clone(), remote_access)
                .with_before_revoke_guard_for_test(before_revoke_tx);
        let service = Arc::new(
            DaemonService::with_sessions(setup, 123, sessions.clone())
                .with_device_management(management),
        );
        let lock = DaemonLock::try_acquire(&state.paths)
            .expect("daemon lock probe")
            .expect("daemon lock");
        let listener = bind_daemon_socket(&state.paths, &lock).expect("local listener");
        let server = tokio::spawn(serve_local_with_limits(
            listener,
            state.paths.uid(),
            service,
            LocalIpcLimits::for_test(Duration::from_secs(10)),
        ));
        Self {
            state,
            lock,
            actor,
            store: handle,
            server,
            access,
            registry,
            sessions,
            before_revoke,
            order,
        }
    }

    fn device_client(&self) -> LocalDeviceClient {
        LocalDeviceClient::new(self.state.paths.socket())
    }

    async fn wait_before_revoke(&mut self, expected: DeviceId) {
        assert_eq!(
            self.before_revoke.recv().await,
            Some(expected),
            "revoke reaches the write-gate boundary"
        );
    }

    async fn stop(self) {
        LocalClient::new(self.state.paths.socket())
            .stop(false)
            .await
            .expect("daemon stop");
        self.server
            .await
            .expect("local listener task")
            .expect("local listener result");
        remove_own_socket(&self.state.paths, &self.lock).expect("remove owned socket");
        self.actor.shutdown();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_orders_started_and_queued_commits_before_close_and_detach() {
    let sessions = SessionFixture::new(ResourceLimits::default()).expect("session fixture");
    let remote_session = sessions.create(1, "remote").expect("remote session");
    let other_session = sessions.create(2, "other").expect("other session");
    let remote_attachment = sessions
        .service
        .prepare_attach(
            AttachmentPrincipal::RemoteEndpoint {
                device_id: REMOTE,
                auth_generation: 1,
            },
            Some(SessionSelector::Id(remote_session.session_id)),
            false,
            false,
            None,
        )
        .expect("remote attachment");
    session_fixture::activate(&remote_attachment).expect("activate remote attachment");
    let other_attachment = sessions
        .service
        .prepare_attach(
            AttachmentPrincipal::RemoteEndpoint {
                device_id: OTHER,
                auth_generation: 1,
            },
            Some(SessionSelector::Id(other_session.session_id)),
            false,
            false,
            None,
        )
        .expect("other attachment");
    session_fixture::activate(&other_attachment).expect("activate other attachment");

    let mut harness = Harness::start(sessions.service.clone(), |store| {
        store
            .authorize_device(REMOTE, "remote", 10)
            .expect("authorize remote");
        store
            .authorize_device(OTHER, "other", 11)
            .expect("authorize other");
        let route = RelayRouteCache {
            relay_hints: vec![RelayHint::new("https://relay.example.test").expect("relay hint")],
            verified_at_unix: 12,
        };
        store
            .upsert_known_device(
                REMOTE,
                &DeviceAlias::new("remote-alias").expect("remote alias"),
                "remote",
                Some(&route),
            )
            .expect("known remote");
    });
    harness.access.set_live(
        REMOTE,
        DeviceLiveObservation {
            online: true,
            active_stream_count: 1,
            remote_attachment_count: 1,
        },
    );

    let old_context = harness
        .registry
        .acquire_commit(REMOTE, generation(1))
        .await
        .expect("old generation starts before revoke");
    let (old_entered_tx, old_entered_rx) = oneshot::channel();
    let (old_release_tx, old_release_rx) = oneshot::channel();
    let old_order = Arc::clone(&harness.order);
    let old_commit = tokio::spawn(async move {
        old_context
            .run(move || {
                old_entered_tx.send(()).expect("signal old commit entry");
                old_release_rx.blocking_recv().expect("release old commit");
                lock(&old_order).push("old_commit");
                Ok::<(), DaemonError>(())
            })
            .await
    });
    old_entered_rx.await.expect("old commit is in flight");

    let socket = harness.state.paths.socket().to_path_buf();
    let revoke = tokio::spawn(async move { LocalDeviceClient::new(socket).revoke(REMOTE).await });
    harness.wait_before_revoke(REMOTE).await;

    let queued_registry = harness.registry.clone();
    let (queued_started_tx, queued_started_rx) = oneshot::channel();
    let queued = tokio::spawn(async move {
        queued_started_tx.send(()).expect("signal queued reader");
        queued_registry
            .acquire_commit(REMOTE, generation(1))
            .await
            .map(|_| ())
            .map_err(|error| error.kind())
    });
    queued_started_rx.await.expect("queued reader starts");

    assert_eq!(
        harness
            .store
            .authorization_snapshot(REMOTE, default_store_deadline())
            .expect("durable snapshot while old commit runs"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: generation(1),
        },
        "the revoke cannot reach SQLite while the old read permit is held"
    );
    assert!(harness.access.evidence().is_empty());
    assert!(!queued.is_finished());

    old_release_tx.send(()).expect("release old commit");
    old_commit
        .await
        .expect("old commit task")
        .expect("old authorized side effect completes");

    let close = harness.access.wait_for_close_count(1).await;
    assert_eq!(
        close,
        CloseEvidence {
            device_id: REMOTE,
            durable: AuthorizationSnapshot {
                status: AuthorizationStatus::Revoked,
                generation: generation(2),
            },
            memory: AuthorizationSnapshot {
                status: AuthorizationStatus::Revoked,
                generation: generation(2),
            },
        },
        "durable commit and registry publication precede remote close"
    );
    assert_eq!(&*lock(&harness.order), &["old_commit", "close"]);
    assert!(
        !queued.is_finished(),
        "write guard is retained across close"
    );
    remote_attachment
        .attachment
        .write_input(b"before-detach\n")
        .expect("matching controller remains until close returns");

    harness.access.release_first_close();
    let revoked = revoke.await.expect("revoke task").expect("revoke succeeds");
    assert_eq!(revoked.auth_status(), AuthorizationStatus::Revoked);
    assert_eq!(revoked.generation(), generation(2));
    assert!(revoked.outbound_known());
    assert_eq!(
        revoked.alias().expect("retained alias").as_str(),
        "remote-alias"
    );
    assert!(revoked.route_verified());
    assert_eq!(
        queued.await.expect("queued reader joins"),
        Err(DomainErrorKind::Unauthorized),
        "a reader queued after the revoke writer cannot commit"
    );
    assert_eq!(
        harness
            .registry
            .acquire_commit(REMOTE, generation(1))
            .await
            .expect_err("new commit is rejected")
            .kind(),
        DomainErrorKind::Unauthorized
    );
    assert_eq!(
        remote_attachment
            .attachment
            .write_input(b"stale")
            .expect_err("matching controller is detached after close")
            .kind(),
        DomainErrorKind::LeaseLost
    );
    other_attachment
        .attachment
        .write_input(b"other-still-live\n")
        .expect("another peer controller remains live");
    assert_eq!(harness.sessions.list().expect("sessions remain").len(), 2);
    assert_eq!(
        harness.registry.snapshot(OTHER).expect("other snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: generation(1),
        }
    );
    let known = harness
        .store
        .known_device(REMOTE, default_store_deadline())
        .expect("known row read")
        .expect("outbound known row remains");
    assert_eq!(known.local_alias.as_str(), "remote-alias");
    assert!(known.route_cache.is_some());

    let repeated = harness
        .device_client()
        .revoke(REMOTE)
        .await
        .expect("repeated revoke is idempotent");
    assert_eq!(repeated.generation(), generation(2));
    assert_eq!(harness.access.evidence().len(), 2);
    assert_eq!(harness.sessions.list().expect("sessions remain").len(), 2);

    let restarted = AuthorizationRegistry::new();
    restarted
        .preload(
            harness
                .store
                .list_authorizations(default_store_deadline())
                .expect("restart preload rows"),
        )
        .expect("restart preload");
    assert_eq!(
        restarted.snapshot(REMOTE).expect("restart snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Revoked,
            generation: generation(2),
        }
    );
    assert_eq!(
        restarted
            .admit(REMOTE)
            .expect_err("revocation survives restart preload")
            .kind(),
        DomainErrorKind::Unauthorized
    );

    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_revoke_preserves_live_state_and_retry_completes_order() {
    let sessions = SessionFixture::new(ResourceLimits::default()).expect("session fixture");
    let summary = sessions.create(1, "kept").expect("session creates");
    let attachment = sessions
        .service
        .prepare_attach(
            AttachmentPrincipal::RemoteEndpoint {
                device_id: REMOTE,
                auth_generation: 1,
            },
            Some(SessionSelector::Id(summary.session_id)),
            false,
            false,
            None,
        )
        .expect("remote attachment");
    session_fixture::activate(&attachment).expect("activate remote attachment");
    let mut harness = Harness::start(sessions.service.clone(), |store| {
        store
            .authorize_device(REMOTE, "remote", 10)
            .expect("authorize remote");
        store
            .upsert_known_device(
                REMOTE,
                &DeviceAlias::new("kept-alias").expect("alias"),
                "remote",
                None,
            )
            .expect("known remote");
    });
    harness.access.set_live(
        REMOTE,
        DeviceLiveObservation {
            online: true,
            active_stream_count: 1,
            remote_attachment_count: 1,
        },
    );
    {
        let connection =
            rusqlite::Connection::open(harness.state.paths.database()).expect("fixture connection");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_revoke_race
                 BEFORE UPDATE OF status ON device_auth
                 WHEN NEW.status=2
                 BEGIN SELECT RAISE(ABORT, 'injected revoke abort'); END;",
            )
            .expect("install revoke failure");
    }

    assert_eq!(
        harness
            .device_client()
            .revoke(REMOTE)
            .await
            .expect_err("database failure is returned")
            .kind(),
        DomainErrorKind::StoreUnavailable
    );
    harness.wait_before_revoke(REMOTE).await;
    let authorized = AuthorizationSnapshot {
        status: AuthorizationStatus::Authorized,
        generation: generation(1),
    };
    assert_eq!(
        harness.registry.snapshot(REMOTE).expect("memory"),
        authorized
    );
    assert_eq!(
        harness
            .store
            .authorization_snapshot(REMOTE, default_store_deadline())
            .expect("durable"),
        authorized
    );
    assert!(harness.access.evidence().is_empty());
    attachment
        .attachment
        .write_input(b"failure-keeps-controller\n")
        .expect("failed revoke does not detach controller");
    assert_eq!(harness.sessions.list().expect("session remains").len(), 1);
    assert!(
        harness
            .store
            .known_device(REMOTE, default_store_deadline())
            .expect("known row read")
            .is_some()
    );

    {
        let connection =
            rusqlite::Connection::open(harness.state.paths.database()).expect("fixture connection");
        connection
            .execute_batch("DROP TRIGGER fail_revoke_race;")
            .expect("remove revoke failure");
    }
    let socket = harness.state.paths.socket().to_path_buf();
    let retry = tokio::spawn(async move { LocalDeviceClient::new(socket).revoke(REMOTE).await });
    harness.wait_before_revoke(REMOTE).await;
    let close = harness.access.wait_for_close_count(1).await;
    assert_eq!(close.durable.status, AuthorizationStatus::Revoked);
    assert_eq!(close.memory, close.durable);
    assert_eq!(close.durable.generation, generation(2));
    attachment
        .attachment
        .write_input(b"retry-before-detach\n")
        .expect("detach remains ordered after close");
    harness.access.release_first_close();
    let revoked = retry.await.expect("retry task").expect("retry succeeds");
    assert_eq!(revoked.generation(), generation(2));
    assert_eq!(
        attachment
            .attachment
            .write_input(b"stale")
            .expect_err("retry detaches controller")
            .kind(),
        DomainErrorKind::LeaseLost
    );
    assert_eq!(harness.sessions.list().expect("session retained").len(), 1);

    let restarted = AuthorizationRegistry::new();
    restarted
        .preload(
            harness
                .store
                .list_authorizations(default_store_deadline())
                .expect("restart rows"),
        )
        .expect("restart preload");
    assert_eq!(
        restarted.snapshot(REMOTE).expect("restart snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Revoked,
            generation: generation(2),
        }
    );

    harness.stop().await;
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
