# Implement worker brief: pairing + device IPC + immediate revoke

Active task: `.trellis/tasks/08-22-transport-auth`

Implement Steps 7–8 of `implement.md` after core/proto, Store/auth, Session principal, and network broker APIs are present. You own:

- new daemon pairing/device/revoke coordinator modules;
- `crates/daemon/src/service.rs` and `local_ipc.rs` pair/device/status integration;
- minimal network broker hooks for pair ALPN and normal confirmation, coordinating rather than duplicating the broker;
- focused pairing, secret, local-device-IPC and revoke-race tests plus real-socket test clients.

Do not add public clap pair/device/connect/session commands, TTY prompt/argv/stdin UX, M7 remote Session adapter/terminal stream, a second frame decoder/runtime/store/authorization registry, SQLite tables, persistent offers, accounts/ACL/audit/events, or Trellis spec edits.

You are not alone in the codebase. Preserve all existing/user edits and adapt to the landed owner APIs. Do not revert unrelated work. Do not commit, push, or merge.

Required behavior:

1. Pair create enforces TTL 1–60m (default 10m), live-offer/replay-cell limit 16, Endpoint bound plus at least one home Relay hint, `SystemRandom` IDs/secrets, monotonic+wall expiry, exact ticket response replay, and zeroized/redacted bearer memory.
2. Pair host runs exact Ready→Consuming→Consumed/Expired state. Invalid proof never consumes; only a valid-proof CAS touches StoreActor; DB failure rolls Ready back; commit publishes consumed before response and retains a tombstone until expiry so no second EndpointId can authorize.
3. Pair ALPN uses full TLS IDs and the approved transcript/HMAC/confirmation. Enforce 8 global/1 peer, 5s first frame, 15s total and 64KiB traffic. Generic peer errors and tracing contain no ticket/secret/proof.
4. Accept reserves a validated alias before network work, uses transient ticket routes, performs pair handshake, then normal `zterm/1` confirmation. Only confirmed host authorization permits local `known_devices` commit/success. Ambiguous or repeated accept may repair via normal auth; it never reopens a consumed offer or invents remote rollback.
5. Add local kinds 12–21 behind the existing same-UID gate and strict unary EOF. Pair branches are async-native; SQLite/Session effects use existing bounded blocking helpers. Borrow first-frame metadata then move the single frame—remove the sensitive generic clone. Zeroize owned ticket/request/reply buffers after decode/write.
6. Device list merges inbound/outbound/live directions; rename changes only outbound alias; revoke changes only inbound authorization. Exact DeviceId only, safe byte-identical retries, no ambiguous selector.
7. Revoke order is owned write permit → FULL SQLite tombstone commit → memory/watch publish/cancel → broker close remote → SessionService detach matching remote principal → exact impact response. DB failure changes none of those. Repeated revoke is idempotent. Session/PTY/other peer/outbound known row remain.
8. Extend typed status/doctor with redacted network/device projection and one human/JSON source. Stopped doctor stays network-passive. CLI help remains free of M8 command surfaces.

Tests must cover one-way pairing, reverse ticket independence, expiry/tamper/wrong ID/secret/replay/concurrent consumption/DB failure/generation exhaustion, dropped PairAccepted and local responses, secret sentinels, real same-UID kinds/deadlines/EOF, deterministic revoke barriers/restart, and existing local Session/CLI/cross-UID regressions. Run Steps 7–8 focused gates before reporting.
