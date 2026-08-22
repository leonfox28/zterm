//! SQLite schema and durable-state acceptance tests.

#[path = "support/state_fixture.rs"]
mod state_fixture;

use zterm_core::{DeviceId, DomainErrorKind};
use zterm_daemon::store::{
    AuthorizationStatus, DeviceMetadata, StateStore, set_test_schema_version,
};
use zterm_platform::user_state::open_append;

use state_fixture::TestState;

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
        1
    );
    assert_eq!(
        store.authorization_status(peer).expect("status reads"),
        Some((AuthorizationStatus::Authorized, 1))
    );
    assert_eq!(store.revoke_device(peer, 12).expect("revoked"), 2);
    assert_eq!(
        store.authorization_status(peer).expect("status reads"),
        Some((AuthorizationStatus::Revoked, 2))
    );
    store
        .upsert_known_device(peer, "phone", "Personal phone", Some((1, b"route", 13)))
        .expect("route cache inserts");
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
