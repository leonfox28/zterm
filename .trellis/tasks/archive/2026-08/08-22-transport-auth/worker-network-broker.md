# Implement worker brief: Endpoint supervisor + route/broker

Active task: `.trellis/tasks/08-22-transport-auth`

Implement Steps 5–6 of `implement.md` after the core/proto and Store/auth foundations are present. You own:

- `crates/daemon/src/identity.rs`, `transport.rs`, `lifecycle.rs`;
- new focused network supervisor, route resolver, connection broker/registry modules and their `lib.rs` declarations;
- the minimal `DaemonService`/typed status plumbing needed for network observation (but not pair/device IPC);
- focused profile, network lifecycle, route, broker, duplicate, stream-limit, authorization-admission and path tests.

Do not implement PairingManager/proofs/offers, device mutation IPC, public CLI commands, M7 Session RPC/terminal streams, or modify Trellis specs/task artifacts. Do not duplicate StoreActor, AuthorizationRegistry, frame decoder, infrastructure profile, or SessionService.

You are not alone in the codebase. Preserve all existing/user edits, especially the two-ALPN profile change, and adapt to the landed Store/auth and Session-principal APIs. Do not revert unrelated work. Do not commit, push, or merge.

Required behavior:

1. The daemon loads its committed identity into exactly one bound Iroh Endpoint. Local socket readiness/Session/status/stop do not wait for Relay/DNS/Pkarr/Internet. Endpoint bind failures publish truthful degraded state and retry 250ms→10s with bounded jitter and the same identity.
2. The network owner survives the existing fatal local-listener rebind loop. Final stop rejects new work, closes connections/handlers, then bounds `Endpoint::close().await` inside the lifecycle deadline. Connection/path loss never calls Session close/PTY signal.
3. Keep exact official/self-hosted profile maps and both ALPNs. Do not call environment-sensitive lookup shortcuts, `online()` as readiness, 0-RTT APIs, or `insert_relay`.
4. `RouteResolver` explicitly tries fresh signed lookup within 2s, then verified v1 SQLite Relay cache, then transient ticket routes. Candidate `EndpointAddr` contains only target ID plus Relay URL; direct IP is never persisted. A successful authenticated application handshake is the only route-verification/last-seen commit point.
5. `ConnectionBroker` has one `PeerSlot` per EndpointId, one outbound singleflight dial, RAII demand, bounded open-stream queue, global/peer limits, and reconnect only while demand exists. Different peers isolate.
6. Normal `zterm/1` performs Hello/Welcome after full TLS authentication. Unknown/revoked inbound is rejected before reading an application frame. Each incoming service stream rechecks the receiving host's authorization generation; known future M7 kinds return `service_not_implemented` only after admission.
7. Duplicate candidates use the core lexicographic `(authenticated initiator ID, random attempt ID)` reducer. Register provisional before business streams, atomically publish winner, close only loser, and never redial merely because a loser closed while another candidate remains.
8. Enforce all approved connection/dial/preauth/stream/handler/queue/frame/deadline limits with semaphores/checked counters. Stalled or malformed streams reset only themselves. Path events update typed redacted observation only.

Tests use task-private two-identity/local candidates and explicit barriers, not public Internet or wall-clock sleeps as concurrency proof. Run all Step 5 and Step 6 focused gates and report any pinned-Iroh API conflict precisely instead of weakening the contract.
