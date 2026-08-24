//! SQLite schema, durable-state, store actor, and device directory acceptance
//! tests.

#[path = "support/state_fixture.rs"]
mod state_fixture;

use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use zterm_core::{
    AuthGeneration, AuthorizationSnapshot, AuthorizationStatus, DeviceAlias, DeviceDisplayName,
    DeviceId, DomainErrorKind, RelayHint,
};
use zterm_daemon::device_directory::DeviceDirectory;
use zterm_daemon::store::{
    DeviceMetadata, RelayRouteCache, RouteCacheDiagnostic, STORE_COMMAND_CAPACITY, StateStore,
    StoreActor, set_test_schema_version,
};
use zterm_platform::user_state::open_append;

use state_fixture::TestState;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

fn generation(value: u64) -> AuthGeneration {
    AuthGeneration::new(value).expect("generation within the SQLite signed ceiling")
}

#[test]
fn schema_inventory_metadata_authorization_and_route_cache_are_bounded() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let mut store = StateStore::open(&state.paths).expect("store opens");
    assert_eq!(
        store.table_names().expect("schema inventory"),
        ["device_auth", "known_devices", "metadata"]
    );

    let own = DeviceId::from_array([1; 32]);
    store
        .ensure_metadata(&DeviceMetadata {
            device_id: own,
            device_name: "host".to_owned(),
            created_at_unix: 10,
        })
        .expect("metadata inserted");
    assert_eq!(
        store
            .metadata()
            .expect("metadata reads")
            .expect("metadata row")
            .device_id,
        own
    );

    let peer = DeviceId::from_array([2; 32]);
    assert_eq!(
        store
            .authorize_device(peer, "phone", 11)
            .expect("authorized"),
        generation(1)
    );
    assert_eq!(
        store.authorization_snapshot(peer).expect("snapshot reads"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: generation(1),
        }
    );
    assert_eq!(
        store.revoke_device(peer, 12).expect("revoked"),
        generation(2)
    );
    assert_eq!(
        store.authorization_snapshot(peer).expect("snapshot reads"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Revoked,
            generation: generation(2),
        }
    );

    let alias = DeviceAlias::new("phone").expect("bounded alias");
    let route = RelayRouteCache {
        relay_hints: vec![RelayHint::new("https://relay.example.com").expect("relay hint")],
        verified_at_unix: 13,
    };
    store
        .upsert_known_device(peer, &alias, "Personal phone", Some(&route))
        .expect("route cache inserts");
    let known = store
        .known_device(peer)
        .expect("known device reads")
        .expect("known device row");
    assert_eq!(known.local_alias, alias);
    assert_eq!(known.remote_name.as_str(), "Personal phone");
    assert_eq!(known.route_cache, Some(route));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(state.paths.database())
                .expect("database metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    drop(store);
    assert_eq!(
        StateStore::open(&state.paths)
            .expect("v1 migration is idempotent")
            .table_names()
            .expect("reopened schema inventory"),
        ["device_auth", "known_devices", "metadata"]
    );
}

#[test]
fn pair_confirmation_preserves_replaces_and_allows_a_new_route_less_row() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor = StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("actor");
    let handle = actor.handle();
    let peer = DeviceId::from_array([0x71; 32]);
    let old_route = RelayRouteCache {
        relay_hints: vec![RelayHint::new("https://old-relay.example.com").expect("old route")],
        verified_at_unix: 10,
    };
    let new_route = RelayRouteCache {
        relay_hints: vec![RelayHint::new("https://new-relay.example.com").expect("new route")],
        verified_at_unix: 20,
    };

    handle
        .confirm_known_device(
            peer,
            DeviceAlias::new("old-name").expect("alias"),
            "Old name",
            Some(old_route.clone()),
            deadline(),
        )
        .expect("initial confirmation stores route");
    handle
        .confirm_known_device(
            peer,
            DeviceAlias::new("direct-name").expect("alias"),
            "Direct name",
            None,
            deadline(),
        )
        .expect("direct-only confirmation preserves route");
    let preserved = handle
        .known_device(peer, deadline())
        .expect("read preserved row")
        .expect("known row");
    assert_eq!(preserved.local_alias.as_str(), "direct-name");
    assert_eq!(preserved.remote_name.as_str(), "Direct name");
    assert_eq!(preserved.route_cache, Some(old_route));

    handle
        .confirm_known_device(
            peer,
            DeviceAlias::new("verified-name").expect("alias"),
            "Verified name",
            Some(new_route.clone()),
            deadline(),
        )
        .expect("verified confirmation replaces route");
    let replaced = handle
        .known_device(peer, deadline())
        .expect("read replaced row")
        .expect("known row");
    assert_eq!(replaced.local_alias.as_str(), "verified-name");
    assert_eq!(replaced.remote_name.as_str(), "Verified name");
    assert_eq!(replaced.route_cache, Some(new_route));

    let direct_only = DeviceId::from_array([0x72; 32]);
    handle
        .confirm_known_device(
            direct_only,
            DeviceAlias::new("direct-only").expect("alias"),
            "Direct only",
            None,
            deadline(),
        )
        .expect("new route-less row is allowed");
    let route_less = handle
        .known_device(direct_only, deadline())
        .expect("read route-less row")
        .expect("known row");
    assert_eq!(route_less.route_cache, None);
    assert_eq!(route_less.route_cache_diagnostic, None);
    actor.shutdown();
}

#[test]
fn pair_confirmation_alias_conflict_rolls_back_name_and_route_atomically() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor = StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("actor");
    let handle = actor.handle();
    let peer = DeviceId::from_array([0x73; 32]);
    let owner = DeviceId::from_array([0x74; 32]);
    let old_route = RelayRouteCache {
        relay_hints: vec![RelayHint::new("https://old-relay.example.com").expect("old route")],
        verified_at_unix: 10,
    };
    let replacement_route = RelayRouteCache {
        relay_hints: vec![
            RelayHint::new("https://replacement-relay.example.com").expect("replacement route"),
        ],
        verified_at_unix: 20,
    };
    handle
        .confirm_known_device(
            peer,
            DeviceAlias::new("retained").expect("alias"),
            "Retained name",
            Some(old_route.clone()),
            deadline(),
        )
        .expect("seed peer");
    handle
        .confirm_known_device(
            owner,
            DeviceAlias::new("claimed").expect("alias"),
            "Alias owner",
            None,
            deadline(),
        )
        .expect("seed alias owner");

    let error = handle
        .confirm_known_device(
            peer,
            DeviceAlias::new("claimed").expect("alias"),
            "Must roll back",
            Some(replacement_route),
            deadline(),
        )
        .expect_err("unique alias conflict");
    assert_eq!(error.kind(), DomainErrorKind::DeviceAliasConflict);

    let retained = handle
        .known_device(peer, deadline())
        .expect("read peer")
        .expect("peer remains");
    assert_eq!(retained.local_alias.as_str(), "retained");
    assert_eq!(retained.remote_name.as_str(), "Retained name");
    assert_eq!(retained.route_cache, Some(old_route));
    let alias_owner = handle
        .known_device(owner, deadline())
        .expect("read alias owner")
        .expect("owner remains");
    assert_eq!(alias_owner.local_alias.as_str(), "claimed");
    assert_eq!(alias_owner.remote_name.as_str(), "Alias owner");
    actor.shutdown();
}

#[test]
fn too_new_schema_is_refused_without_recreation() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    drop(StateStore::open(&state.paths).expect("v1 store"));
    set_test_schema_version(state.paths.database(), 99).expect("fixture schema version");
    let before = std::fs::read(state.paths.database()).expect("too-new snapshot");
    let error = match StateStore::open(&state.paths) {
        Ok(_) => panic!("too-new schema accepted"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), DomainErrorKind::SchemaTooNew);
    assert_eq!(
        std::fs::read(state.paths.database()).expect("too-new database retained"),
        before
    );
}

#[test]
fn read_only_observation_refuses_old_schema_without_migrating_it() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    drop(open_append(state.paths.database(), state.paths.uid()).expect("database file"));

    let error = match StateStore::open_read_only(&state.paths) {
        Ok(_) => panic!("old schema accepted by read-only observation"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), DomainErrorKind::MigrationFailed);

    let connection = rusqlite::Connection::open(state.paths.database()).expect("inspect database");
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("schema version");
    let tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .expect("table count");
    assert_eq!(version, 0);
    assert_eq!(tables, 0);
}

#[test]
fn migration_failure_is_typed_and_transactionally_preserves_v0() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    drop(open_append(state.paths.database(), state.paths.uid()).expect("database file"));
    let connection = rusqlite::Connection::open(state.paths.database()).expect("fixture database");
    connection
        .execute_batch("CREATE TABLE metadata (marker TEXT NOT NULL); INSERT INTO metadata VALUES ('retained');")
        .expect("conflicting v0 fixture");
    drop(connection);

    let error = match StateStore::open(&state.paths) {
        Ok(_) => panic!("conflicting migration succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), DomainErrorKind::MigrationFailed);

    let connection = rusqlite::Connection::open(state.paths.database()).expect("inspect database");
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("schema version");
    let marker: String = connection
        .query_row("SELECT marker FROM metadata", [], |row| row.get(0))
        .expect("retained v0 row");
    assert_eq!(version, 0);
    assert_eq!(marker, "retained");
}

#[test]
fn generation_transitions_are_checked_and_revoke_is_idempotent() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let mut store = StateStore::open(&state.paths).expect("store opens");
    let peer = DeviceId::from_array([2; 32]);

    assert_eq!(
        store.authorize_device(peer, "phone", 1).expect("authorize"),
        generation(1)
    );
    let row = store
        .list_authorizations()
        .expect("list after authorize")
        .remove(0);
    assert_eq!(row.status, AuthorizationStatus::Authorized);
    assert_eq!(row.generation, generation(1));
    assert_eq!(row.paired_at_unix, 1);
    assert_eq!(row.revoked_at_unix, None);

    // Re-authorizing the same device always advances and clears any tombstone.
    assert_eq!(
        store
            .authorize_device(peer, "phone", 2)
            .expect("re-authorize"),
        generation(2)
    );
    let row = store
        .list_authorizations()
        .expect("list after reauthorize")
        .remove(0);
    assert_eq!(row.status, AuthorizationStatus::Authorized);
    assert_eq!(row.generation, generation(2));
    assert_eq!(row.paired_at_unix, 2);
    assert_eq!(row.revoked_at_unix, None);
    assert_eq!(
        store.authorization_snapshot(peer).expect("snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: generation(2),
        }
    );

    // First revoke advances and commits a tombstone.
    assert_eq!(store.revoke_device(peer, 3).expect("revoke"), generation(3));
    let row = store
        .list_authorizations()
        .expect("list after revoke")
        .remove(0);
    assert_eq!(row.status, AuthorizationStatus::Revoked);
    assert_eq!(row.generation, generation(3));
    assert_eq!(row.paired_at_unix, 2);
    assert_eq!(row.revoked_at_unix, Some(3));
    assert_eq!(
        store.authorization_snapshot(peer).expect("snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Revoked,
            generation: generation(3),
        }
    );

    // Repeated revoke is idempotent and does not advance.
    assert_eq!(
        store.revoke_device(peer, 4).expect("repeat revoke"),
        generation(3)
    );
    let row = store
        .list_authorizations()
        .expect("list after repeated revoke")
        .remove(0);
    assert_eq!(row.status, AuthorizationStatus::Revoked);
    assert_eq!(row.generation, generation(3));
    assert_eq!(row.paired_at_unix, 2);
    assert_eq!(row.revoked_at_unix, Some(3));
    assert_eq!(
        store.authorization_snapshot(peer).expect("snapshot"),
        AuthorizationSnapshot {
            status: AuthorizationStatus::Revoked,
            generation: generation(3),
        }
    );

    // A fresh authorization after revoke advances again.
    assert_eq!(
        store
            .authorize_device(peer, "phone", 5)
            .expect("re-authorize after revoke"),
        generation(4)
    );
    let row = store
        .list_authorizations()
        .expect("list after post-revoke authorize")
        .remove(0);
    assert_eq!(row.status, AuthorizationStatus::Authorized);
    assert_eq!(row.generation, generation(4));
    assert_eq!(row.paired_at_unix, 5);
    assert_eq!(row.revoked_at_unix, None);

    // Revoking a device with no record is typed, not a wrap or a no-op.
    let ghost = DeviceId::from_array([9; 32]);
    let error = store.revoke_device(ghost, 6).expect_err("missing revoke");
    assert_eq!(error.kind(), DomainErrorKind::DeviceNotFound);
}

#[test]
fn generation_exhaustion_refuses_mutation_without_wrapping() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let mut store = StateStore::open(&state.paths).expect("store opens");
    let peer = DeviceId::from_array([2; 32]);
    store.authorize_device(peer, "phone", 1).expect("authorize");

    // Force the durable generation to the signed 64-bit ceiling.
    {
        let connection =
            rusqlite::Connection::open(state.paths.database()).expect("fixture connection");
        connection
            .execute(
                "UPDATE device_auth SET generation=?1 WHERE endpoint_id=?2",
                rusqlite::params![i64::MAX, peer.as_bytes().as_slice()],
            )
            .expect("seed ceiling generation");
    }

    let ceiling = generation(AuthGeneration::SQLITE_MAX);
    let authorized_ceiling = AuthorizationSnapshot {
        status: AuthorizationStatus::Authorized,
        generation: ceiling,
    };

    // Both advance paths refuse to wrap and leave the durable row unchanged.
    assert_eq!(
        store
            .authorize_device(peer, "phone", 2)
            .expect_err("authorize at ceiling")
            .kind(),
        DomainErrorKind::StoreUnavailable
    );
    assert_eq!(
        store.authorization_snapshot(peer).expect("snapshot"),
        authorized_ceiling
    );
    assert_eq!(
        store
            .revoke_device(peer, 3)
            .expect_err("revoke at ceiling")
            .kind(),
        DomainErrorKind::StoreUnavailable
    );
    assert_eq!(
        store.authorization_snapshot(peer).expect("snapshot"),
        authorized_ceiling
    );
}

#[test]
fn store_handle_deadline_never_executes_expired_side_effects_and_shutdown_joins_once() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let store = StateStore::open(&state.paths).expect("store opens");
    let actor = StoreActor::start(store).expect("actor starts");
    let handle = actor.handle();
    let peer = DeviceId::from_array([5; 32]);

    // A mutation whose deadline already elapsed is never executed.
    let expired = handle.authorize(peer, "phone", 1, Instant::now() - Duration::from_secs(1));
    assert_eq!(
        expired.expect_err("expired deadline").kind(),
        DomainErrorKind::DeadlineExceeded
    );
    assert_eq!(
        handle
            .authorization_snapshot(peer, deadline())
            .expect("snapshot"),
        AuthorizationSnapshot::none()
    );

    // A live mutation executes through the actor.
    assert_eq!(
        handle
            .authorize(peer, "phone", 1, deadline())
            .expect("authorize"),
        generation(1)
    );

    // A blocked actor still fails fast for an already-expired caller.
    let (entered, entered_rx) = mpsc::sync_channel(1);
    let (release, release_rx) = mpsc::channel();
    let blocker = {
        let handle = handle.clone();
        thread::spawn(move || handle.block_for_test(deadline(), entered, release_rx))
    };
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("store thread blocks");
    let start = Instant::now();
    let blocked = handle.metadata(Instant::now() - Duration::from_secs(1));
    assert_eq!(
        blocked.expect_err("expired while blocked").kind(),
        DomainErrorKind::DeadlineExceeded
    );
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "expired caller must not block behind the saturated actor"
    );
    drop(release);
    blocker
        .join()
        .expect("blocker joins")
        .expect("block_for_test completes");

    // The owner joins exactly once; late calls observe the stopped actor.
    actor.shutdown();
    assert_eq!(
        handle
            .metadata(deadline())
            .expect_err("post-shutdown call")
            .kind(),
        DomainErrorKind::StoreUnavailable
    );
}

#[test]
fn store_response_disconnect_preserves_pre_start_vs_started_ambiguity() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor = StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("actor");
    let handle = actor.handle();

    assert_eq!(
        handle
            .disconnect_response_for_test(deadline(), false)
            .expect_err("pre-start response loss")
            .kind(),
        DomainErrorKind::StoreUnavailable
    );
    assert_eq!(
        handle
            .disconnect_response_for_test(Instant::now() - Duration::from_secs(1), true,)
            .expect_err("start gate expires before effect")
            .kind(),
        DomainErrorKind::DeadlineExceeded
    );
    assert_eq!(
        handle
            .disconnect_response_for_test(deadline(), true)
            .expect_err("started response loss is ambiguous")
            .kind(),
        DomainErrorKind::OperationOutcomeUnknown
    );
    actor.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn store_mailbox_is_exactly_bounded_without_starving_the_runtime() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor =
        StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("actor starts");
    let handle = actor.handle();

    // Occupy the actor itself, then deterministically observe every one of the
    // 64 mailbox slots being admitted.
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::channel();
    let blocker_handle = handle.clone();
    let blocker =
        thread::spawn(move || blocker_handle.block_for_test(deadline(), entered_tx, release_rx));
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("actor is blocked");

    let (queued_tx, queued_rx) = mpsc::channel();
    let mut queued_workers = Vec::with_capacity(STORE_COMMAND_CAPACITY);
    for _ in 0..STORE_COMMAND_CAPACITY {
        let queued_handle = handle.clone();
        let queued_tx = queued_tx.clone();
        queued_workers.push(thread::spawn(move || {
            queued_handle.metadata_queued_for_test(deadline(), queued_tx)
        }));
    }
    drop(queued_tx);
    for _ in 0..STORE_COMMAND_CAPACITY {
        queued_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("one exact mailbox slot admitted");
    }

    // The next runtime-facing wait expires on the blocking pool. A local task
    // on this single-thread runtime still gets scheduled, proving the bounded
    // queue wait did not run inline on the async executor.
    let request_deadline = Instant::now() + Duration::from_millis(50);
    let blocked_wait =
        handle.run_blocking_until(request_deadline, |store, deadline| store.metadata(deadline));
    let local_progress = async {
        tokio::task::yield_now().await;
        "runtime-progress"
    };
    let (blocked_result, progress) = tokio::join!(blocked_wait, local_progress);
    assert_eq!(progress, "runtime-progress");
    assert_eq!(
        blocked_result
            .expect_err("the sixty-fifth queued request expires")
            .kind(),
        DomainErrorKind::DeadlineExceeded
    );

    release_tx.send(()).expect("release actor");
    tokio::task::spawn_blocking(move || {
        blocker
            .join()
            .expect("blocker joins")
            .expect("blocker completes");
        for worker in queued_workers {
            worker
                .join()
                .expect("queued worker joins")
                .expect("queued metadata completes");
        }
    })
    .await
    .expect("join blocking workers");
    actor.shutdown();
}

#[test]
fn directory_merge_does_not_conflate_directions() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor = StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("actor");
    let directory = DeviceDirectory::new(actor.handle());
    let deadline = deadline();

    // Inbound-only device: authorized, but absent from the address book.
    let inbound = DeviceId::from_array([0xa1; 32]);
    actor
        .handle()
        .authorize(inbound, "phone", 1, deadline)
        .expect("authorize inbound");

    // Outbound-only device: known with a verified route, but never authorized.
    let outbound = DeviceId::from_array([0xa2; 32]);
    let alias = DeviceAlias::new("work-laptop").expect("alias");
    let route = RelayRouteCache {
        relay_hints: vec![RelayHint::new("https://relay.example.com").expect("relay hint")],
        verified_at_unix: 2,
    };
    actor
        .handle()
        .upsert_known_device(
            outbound,
            alias.clone(),
            "Work laptop".to_owned(),
            Some(route),
            deadline,
        )
        .expect("upsert outbound");

    // Two-direction device.
    let both = DeviceId::from_array([0xa3; 32]);
    let both_alias = DeviceAlias::new("desktop").expect("alias");
    actor
        .handle()
        .authorize(both, "desktop", 3, deadline)
        .expect("authorize both");
    actor
        .handle()
        .upsert_known_device(
            both,
            both_alias.clone(),
            "Desktop".to_owned(),
            None,
            deadline,
        )
        .expect("upsert both");

    let projections = directory.list(deadline).expect("merged list");
    assert_eq!(projections.len(), 3);

    let inbound_projection = projections
        .iter()
        .find(|p| p.device_id == inbound)
        .expect("inbound projection");
    assert_eq!(inbound_projection.alias, None);
    assert_eq!(inbound_projection.remote_name, None);
    assert!(!inbound_projection.route_verified);
    assert_eq!(
        inbound_projection.auth,
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: generation(1),
        }
    );

    let outbound_projection = projections
        .iter()
        .find(|p| p.device_id == outbound)
        .expect("outbound projection");
    assert_eq!(outbound_projection.alias.as_ref(), Some(&alias));
    assert_eq!(
        outbound_projection.remote_name.as_deref(),
        Some("Work laptop")
    );
    assert!(outbound_projection.route_verified);
    assert_eq!(outbound_projection.auth, AuthorizationSnapshot::none());

    let both_projection = projections
        .iter()
        .find(|p| p.device_id == both)
        .expect("both projection");
    assert_eq!(both_projection.alias.as_ref(), Some(&both_alias));
    assert_eq!(
        both_projection.auth,
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: generation(1),
        }
    );

    actor.shutdown();
}

#[test]
fn session_target_resolution_is_exact_directional_and_frozen_across_rename() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor = StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("actor");
    let handle = actor.handle();
    let directory = DeviceDirectory::new(handle.clone());
    let deadline = deadline();

    let outbound = DeviceId::from_array([0xc1; 32]);
    handle
        .upsert_known_device(
            outbound,
            DeviceAlias::new("Workstation").expect("alias"),
            "Workstation".to_owned(),
            None,
            deadline,
        )
        .expect("outbound row");
    let inbound_only = DeviceId::from_array([0xc2; 32]);
    handle
        .authorize(inbound_only, "controller", 1, deadline)
        .expect("inbound authorization");
    let unknown = DeviceId::from_array([0xc3; 32]);

    let local = directory
        .resolve_session_target("local", deadline)
        .expect("reserved local target");
    assert!(local.is_local());
    let alias_target = directory
        .resolve_session_target("Workstation", deadline)
        .expect("exact case-sensitive alias");
    assert_eq!(alias_target.device_id(), Some(outbound));
    assert_eq!(
        directory
            .resolve_session_target(&outbound.to_string(), deadline)
            .expect("canonical full device ID")
            .device_id(),
        Some(outbound)
    );

    for invalid in [
        outbound.to_string().to_uppercase(),
        outbound.to_string()[..12].to_owned(),
    ] {
        assert_eq!(
            directory
                .resolve_session_target(&invalid, deadline)
                .expect_err("non-canonical IDs are rejected")
                .kind(),
            DomainErrorKind::InvalidTargetSelector
        );
    }
    assert_eq!(
        directory
            .resolve_session_target("workstation", deadline)
            .expect_err("alias comparison is case-sensitive")
            .kind(),
        DomainErrorKind::DeviceNotFound
    );
    assert_eq!(
        directory
            .resolve_session_target(&inbound_only.to_string(), deadline)
            .expect_err("inbound permission is not an outbound address-book row")
            .kind(),
        DomainErrorKind::OutboundDirectionDenied
    );
    assert_eq!(
        directory
            .resolve_session_target(&unknown.to_string(), deadline)
            .expect_err("unknown canonical ID")
            .kind(),
        DomainErrorKind::DeviceNotFound
    );

    directory
        .rename(
            outbound,
            DeviceAlias::new("renamed").expect("renamed alias"),
            deadline,
        )
        .expect("rename outbound alias");
    assert_eq!(
        alias_target.device_id(),
        Some(outbound),
        "the consumed resolved target contains no mutable alias"
    );
    assert_eq!(
        directory
            .resolve_session_target("Workstation", deadline)
            .expect_err("old alias no longer resolves")
            .kind(),
        DomainErrorKind::DeviceNotFound
    );
    assert_eq!(
        directory
            .resolve_session_target("renamed", deadline)
            .expect("new exact alias")
            .device_id(),
        Some(outbound)
    );

    let ambiguous_alias_owner = DeviceId::from_array([0xc4; 32]);
    handle
        .upsert_known_device(
            ambiguous_alias_owner,
            DeviceAlias::new(unknown.to_string()).expect("ID-shaped alias"),
            "Ambiguous".to_owned(),
            None,
            deadline,
        )
        .expect("ID-shaped alias row");
    assert_eq!(
        directory
            .resolve_session_target(&unknown.to_string(), deadline)
            .expect_err("canonical ID and another device alias are ambiguous")
            .kind(),
        DomainErrorKind::InvalidTargetSelector
    );

    actor.shutdown();
}

#[test]
fn rename_touches_only_outbound_and_revoke_only_inbound() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor = StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("actor");
    let directory = DeviceDirectory::new(actor.handle());
    let deadline = deadline();

    let device = DeviceId::from_array([0xb1; 32]);
    actor
        .handle()
        .authorize(device, "peer", 1, deadline)
        .expect("authorize");
    actor
        .handle()
        .upsert_known_device(
            device,
            DeviceAlias::new("old").expect("alias"),
            "Peer".to_owned(),
            None,
            deadline,
        )
        .expect("upsert");

    // Rename changes only the outbound alias, never the inbound generation.
    directory
        .rename(device, DeviceAlias::new("new").expect("alias"), deadline)
        .expect("rename");
    let renamed = directory.list(deadline).expect("list");
    let projection = renamed
        .iter()
        .find(|p| p.device_id == device)
        .expect("projection");
    assert_eq!(projection.alias.as_ref().expect("alias").as_str(), "new");
    assert_eq!(
        projection.auth,
        AuthorizationSnapshot {
            status: AuthorizationStatus::Authorized,
            generation: generation(1),
        }
    );

    // Revoke changes only the inbound status, never the outbound alias.
    actor.handle().revoke(device, 2, deadline).expect("revoke");
    let revoked = directory.list(deadline).expect("list");
    let projection = revoked
        .iter()
        .find(|p| p.device_id == device)
        .expect("projection");
    assert_eq!(projection.alias.as_ref().expect("alias").as_str(), "new");
    assert_eq!(
        projection.auth,
        AuthorizationSnapshot {
            status: AuthorizationStatus::Revoked,
            generation: generation(2),
        }
    );

    // Renaming a device with no outbound row is device_not_found.
    let ghost = DeviceId::from_array([0xb2; 32]);
    let error = directory
        .rename(ghost, DeviceAlias::new("ghost").expect("alias"), deadline)
        .expect_err("rename missing");
    assert_eq!(error.kind(), DomainErrorKind::DeviceNotFound);

    actor.shutdown();
}

#[test]
fn concurrent_alias_reservation_has_one_winner() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor = StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("actor");
    let directory = DeviceDirectory::new(actor.handle());
    let alias = DeviceAlias::new("shared").expect("alias");
    let device_a = DeviceId::from_array([0xc1; 32]);
    let device_b = DeviceId::from_array([0xc2; 32]);

    let barrier = Arc::new(Barrier::new(3));
    let (result_tx, result_rx) = mpsc::channel();
    let reservation_deadline = deadline();
    let attempt = |device: DeviceId, directory: DeviceDirectory| {
        let barrier = Arc::clone(&barrier);
        let result_tx = result_tx.clone();
        let alias = alias.clone();
        thread::spawn(move || {
            barrier.wait();
            let reservation = directory.reserve_alias(device, alias.clone(), reservation_deadline);
            result_tx
                .send(
                    reservation
                        .as_ref()
                        .map(|_| ())
                        .map_err(|error| error.kind()),
                )
                .expect("send result");
            barrier.wait();
            drop(reservation);
        })
    };
    let thread_a = attempt(device_a, directory.clone());
    let thread_b = attempt(device_b, directory.clone());

    barrier.wait();
    let mut results = [result_rx.recv().expect("a"), result_rx.recv().expect("b")];
    results.sort_by_key(|result| result.is_ok());
    assert_eq!(results[0], Err(DomainErrorKind::DeviceAliasConflict));
    assert_eq!(results[1], Ok(()));
    barrier.wait();

    thread_a.join().expect("join a");
    thread_b.join().expect("join b");
    actor.shutdown();
}

#[test]
fn reserve_alias_rejects_durable_owner_but_allows_the_owner() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor = StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("actor");
    let directory = DeviceDirectory::new(actor.handle());
    let device_a = DeviceId::from_array([0xf3; 32]);
    let device_b = DeviceId::from_array([0xf4; 32]);
    let alias = DeviceAlias::new("committed").expect("alias");
    let deadline = deadline();

    // Device A commits the alias durably.
    actor
        .handle()
        .upsert_known_device(device_a, alias.clone(), "A".to_owned(), None, deadline)
        .expect("commit alias");

    // A different device cannot reserve the durably-owned alias.
    assert_eq!(
        directory
            .reserve_alias(device_b, alias.clone(), deadline)
            .expect_err("durable owner")
            .kind(),
        DomainErrorKind::DeviceAliasConflict
    );

    // The owner may re-reserve its own alias.
    directory
        .reserve_alias(device_a, alias.clone(), deadline)
        .expect("owner re-reserves");
    actor.shutdown();
}

#[test]
fn same_device_reservations_are_ref_counted() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor = StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("actor");
    let directory = DeviceDirectory::new(actor.handle());
    let device_a = DeviceId::from_array([0xf5; 32]);
    let device_b = DeviceId::from_array([0xf6; 32]);
    let alias = DeviceAlias::new("shared").expect("alias");
    let deadline = deadline();

    let first = directory
        .reserve_alias(device_a, alias.clone(), deadline)
        .expect("first reservation");
    let second = directory
        .reserve_alias(device_a, alias.clone(), deadline)
        .expect("same-device second reservation");

    // Dropping the first guard must not release the second reservation.
    drop(first);
    assert_eq!(
        directory
            .reserve_alias(device_b, alias.clone(), deadline)
            .expect_err("still reserved by device A")
            .kind(),
        DomainErrorKind::DeviceAliasConflict
    );

    // Dropping the last guard releases the alias for another device.
    drop(second);
    directory
        .reserve_alias(device_b, alias.clone(), deadline)
        .expect("device B can now reserve");
    actor.shutdown();
}

#[test]
fn directory_owns_explicit_and_default_alias_selection() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let actor =
        StoreActor::start(StateStore::open(&state.paths).expect("store")).expect("actor starts");
    let directory = DeviceDirectory::new(actor.handle());
    let first_id = DeviceId::from_array([0x21; 32]);
    let second_id = DeviceId::from_array([0x22; 32]);
    let remote_name = DeviceDisplayName::new("phone").expect("display name");
    let first = directory
        .reserve_selected_alias(first_id, &remote_name, None, deadline())
        .expect("clean remote name is preferred");
    assert_eq!(first.alias().as_str(), "phone");

    // A simultaneous default for another device falls back to the stable
    // endpoint suffix, while an explicit conflict remains an error.
    let second = directory
        .reserve_selected_alias(second_id, &remote_name, None, deadline())
        .expect("default alias disambiguates");
    assert_eq!(second.alias().as_str(), "phone-22222222");
    assert_eq!(
        directory
            .reserve_selected_alias(
                second_id,
                &remote_name,
                Some(DeviceAlias::new("phone").expect("explicit alias")),
                deadline(),
            )
            .expect_err("explicit conflict does not silently rename")
            .kind(),
        DomainErrorKind::DeviceAliasConflict
    );

    let reserved_name = DeviceDisplayName::new("local").expect("display name");
    let reserved = directory
        .reserve_selected_alias(
            DeviceId::from_array([0x23; 32]),
            &reserved_name,
            None,
            deadline(),
        )
        .expect("reserved local name is disambiguated");
    assert_eq!(reserved.alias().as_str(), "local-23232323");

    drop((first, second, reserved));
    actor.shutdown();
}

#[test]
fn route_cache_columns_are_validated_together() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let mut store = StateStore::open(&state.paths).expect("store opens");
    let peer = DeviceId::from_array([0xd1; 32]);
    let alias = DeviceAlias::new("peer").expect("alias");
    let route = RelayRouteCache {
        relay_hints: vec![RelayHint::new("https://relay.example.com").expect("relay hint")],
        verified_at_unix: 13,
    };
    store
        .upsert_known_device(peer, &alias, "Peer", Some(&route))
        .expect("upsert");
    assert!(store.known_device(peer).expect("reads").is_some());

    // An unsupported version is ignored as a route, but remains structurally
    // observable while the known-device row itself stays usable.
    {
        let connection =
            rusqlite::Connection::open(state.paths.database()).expect("fixture connection");
        connection
            .execute(
                "UPDATE known_devices SET route_cache_version=99 WHERE endpoint_id=?1",
                rusqlite::params![peer.as_bytes().as_slice()],
            )
            .expect("corrupt version column");
    }
    let unknown = store
        .known_device(peer)
        .expect("known-device row remains readable")
        .expect("known-device row remains present");
    assert_eq!(unknown.remote_name.as_str(), "Peer");
    assert_eq!(unknown.route_cache, None);
    assert_eq!(
        unknown.route_cache_diagnostic,
        Some(RouteCacheDiagnostic::UnsupportedVersion { actual: 99 })
    );

    // A partial row (version present but blob missing) is also corrupt.
    {
        let connection =
            rusqlite::Connection::open(state.paths.database()).expect("fixture connection");
        connection
            .execute(
                "UPDATE known_devices SET route_cache_version=1, route_cache=NULL WHERE endpoint_id=?1",
                rusqlite::params![peer.as_bytes().as_slice()],
            )
            .expect("partial cache row");
    }
    assert_eq!(
        store
            .known_device(peer)
            .expect_err("partial cache row")
            .kind(),
        DomainErrorKind::StoreUnavailable
    );
}

#[test]
fn persisted_rows_reject_invalid_names_timestamps_and_tombstones() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let mut store = StateStore::open(&state.paths).expect("store opens");
    let own = DeviceId::from_array([0xe2; 32]);
    store
        .ensure_metadata(&DeviceMetadata {
            device_id: own,
            device_name: "host".to_owned(),
            created_at_unix: 1,
        })
        .expect("metadata");
    let peer = DeviceId::from_array([0xe3; 32]);
    store.authorize_device(peer, "peer", 2).expect("authorize");
    let route = RelayRouteCache {
        relay_hints: vec![RelayHint::new("https://relay.example.com").expect("relay")],
        verified_at_unix: 3,
    };
    store
        .upsert_known_device(
            peer,
            &DeviceAlias::new("peer").expect("alias"),
            "Peer",
            Some(&route),
        )
        .expect("known device");

    let connection =
        rusqlite::Connection::open(state.paths.database()).expect("fixture connection");
    let auth_error = |store: &StateStore, label: &str| {
        assert_eq!(
            store.list_authorizations().expect_err(label).kind(),
            DomainErrorKind::StoreUnavailable
        );
    };

    connection
        .execute(
            "UPDATE device_auth SET display_name=?1 WHERE endpoint_id=?2",
            rusqlite::params!["bad\nname", peer.as_bytes().as_slice()],
        )
        .expect("corrupt auth name");
    auth_error(&store, "invalid authorization display name");
    connection
        .execute(
            "UPDATE device_auth SET display_name='peer', paired_at_unix=-1 WHERE endpoint_id=?1",
            [peer.as_bytes().as_slice()],
        )
        .expect("corrupt paired timestamp");
    auth_error(&store, "negative pairing timestamp");
    connection
        .execute(
            "UPDATE device_auth SET paired_at_unix=2, revoked_at_unix=4 WHERE endpoint_id=?1",
            [peer.as_bytes().as_slice()],
        )
        .expect("corrupt authorized tombstone");
    auth_error(&store, "authorized row with tombstone");
    connection
        .execute(
            "UPDATE device_auth SET status=2, revoked_at_unix=NULL WHERE endpoint_id=?1",
            [peer.as_bytes().as_slice()],
        )
        .expect("corrupt revoked tombstone");
    auth_error(&store, "revoked row without tombstone");
    connection
        .execute(
            "UPDATE device_auth SET status=1, last_seen_at_unix=-1 WHERE endpoint_id=?1",
            [peer.as_bytes().as_slice()],
        )
        .expect("corrupt last-seen timestamp");
    auth_error(&store, "negative last-seen timestamp");
    connection
        .execute(
            "UPDATE device_auth SET last_seen_at_unix=NULL WHERE endpoint_id=?1",
            [peer.as_bytes().as_slice()],
        )
        .expect("restore authorization");

    connection
        .execute(
            "UPDATE known_devices SET remote_name='' WHERE endpoint_id=?1",
            [peer.as_bytes().as_slice()],
        )
        .expect("corrupt remote name");
    assert_eq!(
        store
            .known_device(peer)
            .expect_err("invalid known-device remote name")
            .kind(),
        DomainErrorKind::StoreUnavailable
    );
    connection
        .execute(
            "UPDATE known_devices SET remote_name='Peer', routes_verified_at_unix=-1 WHERE endpoint_id=?1",
            [peer.as_bytes().as_slice()],
        )
        .expect("corrupt route timestamp");
    assert_eq!(
        store
            .known_device(peer)
            .expect_err("negative route timestamp")
            .kind(),
        DomainErrorKind::StoreUnavailable
    );

    connection
        .execute("UPDATE metadata SET device_name='' WHERE singleton=1", [])
        .expect("corrupt metadata name");
    assert_eq!(
        store.metadata().expect_err("invalid metadata name").kind(),
        DomainErrorKind::StoreUnavailable
    );
    connection
        .execute(
            "UPDATE metadata SET device_name='host', created_at_unix=-1 WHERE singleton=1",
            [],
        )
        .expect("corrupt metadata timestamp");
    assert_eq!(
        store
            .metadata()
            .expect_err("negative metadata timestamp")
            .kind(),
        DomainErrorKind::StoreUnavailable
    );
}

#[test]
fn reauthorize_updates_pairing_timestamp() {
    let state = TestState::new();
    state.paths.prepare_state_directories().expect("state dirs");
    let mut store = StateStore::open(&state.paths).expect("store opens");
    let peer = DeviceId::from_array([0xe1; 32]);

    store
        .authorize_device(peer, "phone", 100)
        .expect("authorize");
    assert_eq!(
        store
            .list_authorizations()
            .expect("list")
            .first()
            .expect("row")
            .paired_at_unix,
        100
    );

    store
        .authorize_device(peer, "phone", 200)
        .expect("re-authorize");
    assert_eq!(
        store
            .list_authorizations()
            .expect("list")
            .first()
            .expect("row")
            .paired_at_unix,
        200
    );
}
