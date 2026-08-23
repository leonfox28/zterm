//! Same-UID local device management and ordered revoke acceptance tests.
//!
//! These fixtures bind only a task-private Unix socket. They never construct
//! an Iroh Endpoint, listen on UDP, or contact public infrastructure.

#[cfg(unix)]
#[path = "support/session_fixture.rs"]
mod session_fixture;
#[cfg(unix)]
#[path = "support/state_fixture.rs"]
mod state_fixture;

#[cfg(unix)]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::future::Future;
#[cfg(unix)]
use std::pin::Pin;
#[cfg(unix)]
use std::sync::{Arc, Mutex, MutexGuard};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use zterm_core::{
    AttachmentPrincipal, AuthorizationSnapshot, AuthorizationStatus, DeviceAlias, DeviceId,
    DomainErrorKind, RelayHint, ResourceLimits, SessionSelector,
};
#[cfg(unix)]
use zterm_daemon::authorization::AuthorizationRegistry;
#[cfg(unix)]
use zterm_daemon::bootstrap::bootstrap;
#[cfg(unix)]
use zterm_daemon::config::{ValidatedInfrastructure, validate_setup_input};
#[cfg(unix)]
use zterm_daemon::device_directory::DeviceDirectory;
#[cfg(unix)]
use zterm_daemon::local_ipc::{
    LocalClient, LocalDeviceClient, LocalIpcLimits, serve_local_with_limits,
};
#[cfg(unix)]
use zterm_daemon::service::{
    DaemonService, DeviceLiveObservation, DeviceManagement, RemoteDeviceAccess,
};
#[cfg(unix)]
use zterm_daemon::session::SessionService;
#[cfg(unix)]
use zterm_daemon::store::{RelayRouteCache, StateStore, StoreActor};
#[cfg(unix)]
use zterm_platform::local_unix::{DaemonLock, bind_daemon_socket, remove_own_socket};
#[cfg(unix)]
use zterm_proto::{DecodedFrame, FrameDecoder, WireKind, encode_message, v1};

#[cfg(unix)]
use session_fixture::Fixture as SessionFixture;
#[cfg(unix)]
use state_fixture::TestState;

#[cfg(unix)]
#[derive(Default)]
struct RecordingRemoteAccess {
    live: Mutex<BTreeMap<DeviceId, DeviceLiveObservation>>,
    closed: Mutex<Vec<DeviceId>>,
}

#[cfg(unix)]
impl RecordingRemoteAccess {
    fn set_live(&self, device_id: DeviceId, observation: DeviceLiveObservation) {
        lock(&self.live).insert(device_id, observation);
    }

    fn closed(&self) -> Vec<DeviceId> {
        lock(&self.closed).clone()
    }
}

#[cfg(unix)]
impl RemoteDeviceAccess for RecordingRemoteAccess {
    fn observe<'a>(
        &'a self,
        device_id: DeviceId,
        _deadline: Instant,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<DeviceLiveObservation, zterm_daemon::error::DaemonError>>
                + Send
                + 'a,
        >,
    > {
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
        _deadline: Instant,
    ) -> Pin<Box<dyn Future<Output = Result<(), zterm_daemon::error::DaemonError>> + Send + 'a>>
    {
        Box::pin(async move {
            lock(&self.closed).push(device_id);
            lock(&self.live).insert(device_id, DeviceLiveObservation::default());
            Ok(())
        })
    }
}

#[cfg(unix)]
struct Harness {
    state: TestState,
    lock: DaemonLock,
    actor: StoreActor,
    server: tokio::task::JoinHandle<Result<(), zterm_daemon::error::DaemonError>>,
    access: Arc<RecordingRemoteAccess>,
    registry: AuthorizationRegistry,
    sessions: SessionService,
}

#[cfg(unix)]
impl Harness {
    fn start(
        sessions: SessionService,
        limits: LocalIpcLimits,
        populate: impl FnOnce(&mut StateStore),
    ) -> Self {
        let state = TestState::new();
        let requested =
            validate_setup_input("device-ipc-host", ValidatedInfrastructure::OfficialN0)
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
        let access = Arc::new(RecordingRemoteAccess::default());
        let remote_access: Arc<dyn RemoteDeviceAccess> = access.clone();
        let management = DeviceManagement::new(handle, directory, registry.clone(), remote_access);
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
            limits,
        ));
        Self {
            state,
            lock,
            actor,
            server,
            access,
            registry,
            sessions,
        }
    }

    fn device_client(&self) -> LocalDeviceClient {
        LocalDeviceClient::new(self.state.paths.socket())
    }

    fn local_client(&self) -> LocalClient {
        LocalClient::new(self.state.paths.socket())
    }

    async fn stop(self) {
        self.local_client().stop(false).await.expect("daemon stop");
        self.server
            .await
            .expect("local listener task")
            .expect("local listener result");
        remove_own_socket(&self.state.paths, &self.lock).expect("remove owned socket");
        self.actor.shutdown();
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_rename_and_revoke_preserve_direction_and_session_ownership() {
    let sessions = SessionFixture::new(ResourceLimits::default()).expect("session fixture");
    let kept = sessions.create(1, "kept").expect("session creates");
    let other = sessions.create(2, "other").expect("other session creates");
    let inbound_and_outbound = DeviceId::from_array([0x11; 32]);
    let outbound_only = DeviceId::from_array([0x22; 32]);
    let inbound_only = DeviceId::from_array([0x33; 32]);
    let remote_principal = AttachmentPrincipal::RemoteEndpoint {
        device_id: inbound_and_outbound,
        auth_generation: 1,
    };
    let remote_attachment = sessions
        .service
        .prepare_attach(
            remote_principal,
            Some(SessionSelector::Id(kept.session_id)),
            false,
            false,
            None,
        )
        .expect("remote attachment");
    session_fixture::activate(&remote_attachment).expect("remote controller activates");
    let other_attachment = sessions
        .service
        .prepare_attach(
            AttachmentPrincipal::RemoteEndpoint {
                device_id: inbound_only,
                auth_generation: 1,
            },
            Some(SessionSelector::Id(other.session_id)),
            false,
            false,
            None,
        )
        .expect("other remote attachment");
    session_fixture::activate(&other_attachment).expect("other remote controller activates");

    let harness = Harness::start(
        sessions.service.clone(),
        LocalIpcLimits::for_test(Duration::from_secs(5)),
        |store| {
            store
                .authorize_device(inbound_and_outbound, "remote-one", 10)
                .expect("authorize first remote");
            store
                .authorize_device(inbound_only, "remote-three", 11)
                .expect("authorize inbound-only remote");
            let route = RelayRouteCache {
                relay_hints: vec![
                    RelayHint::new("https://relay.example.test").expect("relay hint"),
                ],
                verified_at_unix: 12,
            };
            store
                .upsert_known_device(
                    inbound_and_outbound,
                    &DeviceAlias::new("one").expect("alias"),
                    "remote-one",
                    Some(&route),
                )
                .expect("known first remote");
            store
                .upsert_known_device(
                    outbound_only,
                    &DeviceAlias::new("two").expect("alias"),
                    "remote-two",
                    None,
                )
                .expect("known outbound-only remote");
        },
    );
    harness.access.set_live(
        inbound_and_outbound,
        DeviceLiveObservation {
            online: true,
            active_stream_count: 2,
            remote_attachment_count: 1,
        },
    );

    let client = harness.device_client();
    let status_before = harness
        .local_client()
        .status()
        .await
        .expect("status before");
    let devices = client.list().await.expect("device list");
    assert_eq!(devices.len(), 3);
    let first = by_id(&devices, inbound_and_outbound);
    assert!(first.outbound_known());
    assert_eq!(first.alias().expect("outbound alias").as_str(), "one");
    assert!(first.route_verified());
    assert_eq!(first.auth_status(), AuthorizationStatus::Authorized);
    assert_eq!(first.generation().get(), 1);
    assert!(first.online());
    assert_eq!(first.active_stream_count(), 2);
    assert_eq!(first.remote_attachment_count(), 1);
    let outbound = by_id(&devices, outbound_only);
    assert!(outbound.outbound_known());
    assert_eq!(outbound.auth_status(), AuthorizationStatus::None);
    assert_eq!(outbound.generation().get(), 0);
    let inbound = by_id(&devices, inbound_only);
    assert!(!inbound.outbound_known());
    assert_eq!(inbound.auth_status(), AuthorizationStatus::Authorized);

    let renamed = client
        .rename(
            inbound_and_outbound,
            &DeviceAlias::new("work-phone").expect("renamed alias"),
        )
        .await
        .expect("device rename");
    assert_eq!(
        renamed.alias().expect("renamed alias").as_str(),
        "work-phone"
    );
    assert_eq!(renamed.auth_status(), AuthorizationStatus::Authorized);
    assert_eq!(renamed.generation().get(), 1);

    let revoked = client
        .revoke(inbound_and_outbound)
        .await
        .expect("device revoke");
    assert!(
        revoked.outbound_known(),
        "revoke retains outbound address book"
    );
    assert_eq!(
        revoked.alias().expect("retained alias").as_str(),
        "work-phone"
    );
    assert!(revoked.route_verified(), "revoke retains verified route");
    assert_eq!(revoked.auth_status(), AuthorizationStatus::Revoked);
    assert_eq!(revoked.generation().get(), 2);
    assert!(!revoked.online());
    assert_eq!(revoked.active_stream_count(), 0);
    assert_eq!(revoked.remote_attachment_count(), 0);
    assert_eq!(harness.access.closed(), vec![inbound_and_outbound]);
    assert_eq!(harness.sessions.list().expect("sessions remain").len(), 2);
    assert_eq!(
        remote_attachment
            .attachment
            .write_input(b"stale")
            .expect_err("revoked controller is detached")
            .kind(),
        DomainErrorKind::LeaseLost
    );
    other_attachment
        .attachment
        .write_input(b"\n")
        .expect("another remote controller remains active");

    let repeated = client
        .revoke(inbound_and_outbound)
        .await
        .expect("repeated revoke is idempotent");
    assert_eq!(repeated.generation().get(), 2);
    assert_eq!(
        harness
            .sessions
            .list()
            .expect("session still remains")
            .len(),
        2
    );
    let after = client.list().await.expect("list after revoke");
    assert_eq!(
        by_id(&after, inbound_only).auth_status(),
        AuthorizationStatus::Authorized,
        "another inbound device is unaffected"
    );
    let status_after = harness.local_client().status().await.expect("status after");
    assert_eq!(status_before.started_at_unix, status_after.started_at_unix);
    assert_eq!(
        status_before.active_session_count,
        status_after.active_session_count
    );

    harness.stop().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alias_selector_and_strict_unary_failures_are_connection_local() {
    let device_one = DeviceId::from_array([0x41; 32]);
    let device_two = DeviceId::from_array([0x42; 32]);
    let harness = Harness::start(
        SessionService::new(DeviceId::from_array([0x40; 32])),
        LocalIpcLimits::for_test(Duration::from_millis(250)),
        |store| {
            store
                .upsert_known_device(
                    device_one,
                    &DeviceAlias::new("one").expect("alias"),
                    "remote-one",
                    None,
                )
                .expect("known first remote");
            store
                .upsert_known_device(
                    device_two,
                    &DeviceAlias::new("two").expect("alias"),
                    "remote-two",
                    None,
                )
                .expect("known second remote");
        },
    );
    let client = harness.device_client();

    assert_eq!(
        client
            .rename(
                device_one,
                &DeviceAlias::new("two").expect("conflicting alias"),
            )
            .await
            .expect_err("alias conflict")
            .kind(),
        DomainErrorKind::DeviceAliasConflict
    );
    assert_eq!(
        client
            .rename(
                DeviceId::from_array([0x99; 32]),
                &DeviceAlias::new("missing").expect("alias"),
            )
            .await
            .expect_err("unknown exact device")
            .kind(),
        DomainErrorKind::DeviceNotFound
    );
    assert_eq!(
        client
            .revoke(DeviceId::from_array([0x98; 32]))
            .await
            .expect_err("unknown inbound authorization")
            .kind(),
        DomainErrorKind::DeviceNotFound
    );

    let invalid_alias = encode_message(
        WireKind::LocalDeviceRenameRequest,
        51,
        100,
        &v1::LocalDeviceRenameRequest {
            device_id: Some(device_one.into()),
            alias: "local".to_owned(),
        },
    )
    .expect("invalid alias request");
    assert_error(
        harness.state.paths.socket(),
        invalid_alias,
        DomainErrorKind::InvalidDeviceAlias,
    )
    .await;

    let invalid_selector = encode_message(
        WireKind::LocalDeviceRevokeRequest,
        52,
        100,
        &v1::LocalDeviceRevokeRequest {
            device_id: Some(v1::DeviceId { value: vec![1] }),
        },
    )
    .expect("invalid selector request");
    assert_error(
        harness.state.paths.socket(),
        invalid_selector,
        DomainErrorKind::MalformedFrame,
    )
    .await;

    let first = encode_message(
        WireKind::LocalDeviceListRequest,
        53,
        100,
        &v1::LocalDeviceListRequest {},
    )
    .expect("first list request");
    let second = encode_message(
        WireKind::LocalDeviceListRequest,
        54,
        100,
        &v1::LocalDeviceListRequest {},
    )
    .expect("second list request");
    let mut extra = first;
    extra.extend_from_slice(&second);
    assert_error(
        harness.state.paths.socket(),
        extra,
        DomainErrorKind::MalformedFrame,
    )
    .await;

    // A complete frame without write-half EOF is not dispatched. The request
    // deadline returns one connection-local error and the listener survives.
    let no_eof = encode_message(
        WireKind::LocalDeviceListRequest,
        55,
        50,
        &v1::LocalDeviceListRequest {},
    )
    .expect("no-EOF request");
    let mut stream = tokio::net::UnixStream::connect(harness.state.paths.socket())
        .await
        .expect("raw no-EOF client");
    stream
        .write_all(&no_eof)
        .await
        .expect("write no-EOF request");
    let response = read_response(&mut stream).await;
    assert_service_error(&response, DomainErrorKind::DeadlineExceeded);

    assert_eq!(
        client.list().await.expect("listener remains healthy").len(),
        2
    );
    harness.stop().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn database_revoke_failure_changes_no_live_or_session_state() {
    let sessions = SessionFixture::new(ResourceLimits::default()).expect("session fixture");
    let kept = sessions.create(1, "kept").expect("session creates");
    let remote = DeviceId::from_array([0x71; 32]);
    let remote_principal = AttachmentPrincipal::RemoteEndpoint {
        device_id: remote,
        auth_generation: 1,
    };
    let attachment = sessions
        .service
        .prepare_attach(
            remote_principal,
            Some(SessionSelector::Id(kept.session_id)),
            false,
            false,
            None,
        )
        .expect("remote attachment");
    session_fixture::activate(&attachment).expect("remote controller activates");
    let harness = Harness::start(
        sessions.service.clone(),
        LocalIpcLimits::for_test(Duration::from_secs(5)),
        |store| {
            store
                .authorize_device(remote, "remote", 10)
                .expect("authorize remote");
        },
    );
    harness.access.set_live(
        remote,
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
                "CREATE TRIGGER fail_local_device_revoke
                 BEFORE UPDATE OF status ON device_auth
                 WHEN NEW.status=2
                 BEGIN SELECT RAISE(ABORT, 'injected revoke abort'); END;",
            )
            .expect("install revoke failure");
    }

    let client = harness.device_client();
    assert_eq!(
        client
            .revoke(remote)
            .await
            .expect_err("injected database failure")
            .kind(),
        DomainErrorKind::StoreUnavailable
    );
    assert_eq!(
        harness.registry.snapshot(remote).expect("memory snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: zterm_core::AuthGeneration::new(1).expect("generation"),
        }
    );
    assert!(harness.access.closed().is_empty());
    attachment
        .attachment
        .write_input(b"\n")
        .expect("controller remains live after failed revoke");
    assert_eq!(harness.sessions.list().expect("session remains").len(), 1);
    let unchanged = client.list().await.expect("list after failed revoke");
    let unchanged = by_id(&unchanged, remote);
    assert_eq!(unchanged.auth_status(), AuthorizationStatus::Authorized);
    assert_eq!(unchanged.generation().get(), 1);
    assert!(unchanged.online());

    {
        let connection =
            rusqlite::Connection::open(harness.state.paths.database()).expect("fixture connection");
        connection
            .execute_batch("DROP TRIGGER fail_local_device_revoke;")
            .expect("remove revoke failure");
    }
    let revoked = client.revoke(remote).await.expect("revoke retry succeeds");
    assert_eq!(revoked.auth_status(), AuthorizationStatus::Revoked);
    assert_eq!(revoked.generation().get(), 2);
    assert_eq!(harness.access.closed(), vec![remote]);
    assert_eq!(harness.sessions.list().expect("session retained").len(), 1);
    assert_eq!(
        attachment
            .attachment
            .write_input(b"stale")
            .expect_err("successful revoke detaches controller")
            .kind(),
        DomainErrorKind::LeaseLost
    );

    harness.stop().await;
}

#[cfg(unix)]
fn by_id(devices: &[zterm_core::DeviceSummary], device_id: DeviceId) -> &zterm_core::DeviceSummary {
    devices
        .iter()
        .find(|device| device.device_id() == device_id)
        .expect("device projection")
}

#[cfg(unix)]
async fn assert_error(socket: &std::path::Path, request: Vec<u8>, expected: DomainErrorKind) {
    let mut stream = tokio::net::UnixStream::connect(socket)
        .await
        .expect("raw device client");
    stream.write_all(&request).await.expect("raw request");
    stream.shutdown().await.expect("finish raw request");
    let response = read_response(&mut stream).await;
    assert_service_error(&response, expected);
}

#[cfg(unix)]
async fn read_response(stream: &mut tokio::net::UnixStream) -> DecodedFrame {
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .await
        .expect("response bytes");
    let mut decoder = FrameDecoder::new();
    let mut frames = decoder.feed(&bytes).expect("response frame");
    decoder.finish().expect("complete response");
    assert_eq!(frames.len(), 1);
    frames.remove(0)
}

#[cfg(unix)]
fn assert_service_error(frame: &DecodedFrame, expected: DomainErrorKind) {
    assert_eq!(frame.kind, WireKind::ServiceErrorResponse);
    let error: v1::ServiceError = frame
        .decode_message(WireKind::ServiceErrorResponse)
        .expect("service error response");
    assert_eq!(error.code, expected.code());
}

#[cfg(unix)]
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
