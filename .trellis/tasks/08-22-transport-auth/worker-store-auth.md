# Implement worker brief: StoreActor + directory + authorization gate

Active task: `.trellis/tasks/08-22-transport-auth`

Implement only Step 3 of `implement.md`. You own:

- `crates/daemon/src/store.rs`;
- new focused daemon modules for the device directory and authorization registry/gate, plus their `crates/daemon/src/lib.rs` declarations;
- `crates/daemon/tests/persistence.rs`, a focused authorization integration test, and minimal shared state fixtures.

Do not modify core/proto sources, Session/local IPC/service/lifecycle/transport code, CLI commands, or Trellis specs/task artifacts. Use the core domain types landed by the core/proto worker; do not create daemon-local duplicates.

You are not alone in the codebase. Preserve all existing/user edits, do not revert unrelated changes, and adapt to concurrently landed core/proto and Session changes. Do not commit, push, or merge.

Required behavior:

1. Keep SQLite `user_version=1` and the exact three-table inventory. Extend existing rows/queries only; no migration, new table, second connection, async SQLite wrapper, audit table, or persistent pairing offer.
2. Replace authorization generation saturation with checked, SQLite-i64-compatible advancement. Authorize always advances. First revoke advances and commits a tombstone; repeated revoke is idempotent and returns the current generation; missing revoke is `device_not_found`.
3. Replace the unbounded runtime StoreActor sender with one capacity-64 `sync_channel`. Expose a cloneable `StoreHandle`; the sole actor owner retains shutdown/join. Commands cover metadata, auth snapshots/list/get/authorize/revoke/last-seen, known-device list/get/upsert/verified route/rename, and alias availability. Each call has an absolute deadline/started gate and never blocks a Tokio runtime inline.
4. Add validated row projections. Corrupt status/generation/identifier/cache data yields typed store errors; checked arithmetic never wraps. Preserve `PRAGMA synchronous=FULL` and Immediate transactions for mutations.
5. Add `DeviceDirectory` as the one owner of directional known/inbound merge and alias reservations shared by accept/rename. Rename touches only `known_devices`; revoke touches only `device_auth`. Exact DeviceId only, with `local` reserved and deterministic default alias from core.
6. Add `AuthorizationRegistry`: startup preload from store; short outer map lock; one fair Tokio owned `RwLock` and watch snapshot per EndpointId. Connection/stream admission checks status/generation. A sensitive commit context holds an owned read permit through the actual provided side-effect closure. Authorize/revoke acquire the owned write permit. Do not close transport or detach Sessions here; expose the ordered primitives to the later revoke coordinator.

Tests must prove schema/table compatibility; exact generation transitions including exhaustion/idempotent revoke; DB failure leaves durable and in-memory state unchanged; inbound/outbound merge does not conflate directions; concurrent alias reservation has one winner; StoreActor bounded callers remain responsive and owner joins once; writer fairness/expected-generation checks use barriers rather than sleep. Run the Step 3 focused gate before reporting.
