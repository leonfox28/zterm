# Research: Step 7 pairing integration after PairingManager foundation

- Query: Identify the exact remaining Step 7 work for pair ALPN hosting, local pair-create/pair-accept IPC, transient routing, normal authorization confirmation, persistence/registry/directory ordering, and daemon lifecycle composition.
- Scope: internal
- Date: 2026-08-23

## Findings

### Current foundation and concurrent work

- `crates/daemon/src/pairing.rs:575` now owns a bounded `PairingManager`. It already provides exact create replay (`create_offer_until`), random nonces, challenge preparation, proof-before-CAS (`try_consume_until`), explicit pre-commit rollback, fail-closed `PairConsumption` drop, and post-StoreActor commit/confirmation (`commit`). Its pure coverage is in `crates/daemon/tests/pairing_manager.rs:170` through `:431`.
- `crates/daemon/src/network.rs:812` already routes the two ALPNs after full TLS. Normal connections enter the broker; pair connections are still closed as unavailable at `:853-861`. The current path does not call an Iroh 0-RTT API.
- `crates/daemon/src/connection_broker.rs:752` and `:777` provide durable and transient normal-ALPN demand. `ConnectionDemand::open_bi` at `:978` waits for a promoted primary plus the generation returned by `ConnectionWelcome`, but there is no wait-only normal-confirmation API and no short-lived pair-ALPN dial API.
- `crates/daemon/src/service.rs:347` now has async dispatch and production device-management wiring. Kinds 16-21 are implemented, while kinds 12-15 are not routed. `crates/daemon/src/local_ipc.rs:312-334` now moves the first unary frame rather than cloning its sensitive payload, and `LocalDeviceClient` exists at `:2103`.
- Production lifecycle now creates exactly one `StoreActor`, `AuthorizationRegistry`, `DeviceDirectory`, broker/network owner, and device-management service (`crates/daemon/src/lifecycle.rs:236-265`). Step 7 must reuse this exact `DeviceDirectory`; it must not construct another reservation map.
- Real-Iroh test fixtures are loopback-only and explicitly ignored/panicked on macOS (`crates/daemon/tests/support/network_fixture.rs:38-55`, `crates/daemon/tests/connection_broker.rs:18-23`). No Endpoint was bound while preparing this research.

### Minimal ownership/composition plan

Keep all Step 7 orchestration in `crates/daemon/src/pairing.rs` by adding one cloneable `PairingService` next to the existing manager. It should own only references to existing owners:

```rust
PairingService {
    manager: PairingManager,
    store: StoreHandle,
    authorization: AuthorizationRegistry,
    directory: DeviceDirectory,       // the lifecycle-created shared instance
    broker: ConnectionBroker,         // sole Endpoint/route owner
    network: NetworkObserver,          // ticket-create route readiness
    identity: ConnectionIdentity,      // local display/version/device ID
    limits: TransportLimits,
    accept_operations: AcceptOperationRegistry,
    shutdown: watch::Receiver<bool>,
}
```

Recommended composition with the fewest constructor changes:

1. `NetworkStartup::prepare` continues constructing the sole broker/Endpoint watch.
2. Lifecycle constructs `PairingService::new(...)` immediately afterward from `network_handle.broker()`, `network_handle.observe()`, the existing store/auth/directory, and the already validated connection identity.
3. Add `NetworkStartup::with_pairing(PairingService) -> Self`; production must install it before `spawn`. The existing network-lifecycle tests may omit it because injected failures never bind.
4. Store the same clone in `DaemonService::with_pairing(...)` for local kinds 12/14.
5. `route_incoming` receives that service and calls `accept_pair_connection(connection)` for `zterm-pair/1`; it never calls `accept_normal`, never creates a peer candidate, and therefore never appears in broker primary metrics.
6. Pairing service shutdown is signaled before broker/Endpoint close and waits for its bounded active operations under the existing absolute network shutdown deadline. Outbound accept cells must observe this signal so detached `tokio::spawn` work cannot outlive the daemon owner.

This preserves the ownership table in `design.md`: PairingManager owns offers, DeviceDirectory owns aliases, StoreActor owns SQLite, AuthorizationRegistry owns the live generation gate, ConnectionBroker owns the Endpoint/normal registry, and SessionService is untouched by pairing.

### Exact missing APIs

#### ConnectionBroker / network

Add crate-private APIs; do not expose the raw Endpoint:

```rust
ConnectionBroker::connect_pair_transient(
    remote: DeviceId,
    routes: Vec<RelayHint>,
    deadline: Instant,
) -> Result<PairConnection, DaemonError>

ConnectionDemand::confirm_authorization(
    &self,
    deadline: Instant,
) -> Result<AuthorizationConfirmation, DaemonError>

AuthorizationConfirmation {
    remote: DeviceId,
    generation: AuthGeneration,
    verified_relay: Option<RelayHint>,
}
```

- `connect_pair_transient` reuses the broker's existing Endpoint watch and `RouteResolver`, connects with `ZTERM_PAIR_ALPN`, waits for full TLS, and verifies `connection.remote_id()` equals the ticket host before returning. It must not register a `PeerSlot` candidate or persist a route.
- `confirm_authorization` refactors the selection loop currently embedded in `open_bi` (`connection_broker.rs:997-1045`). It waits for the promoted normal candidate and `remote_acceptance`, but opens no application stream. This avoids creating an empty stream that the host would classify as a malformed M7 request.
- Return the actual candidate relay that survived TLS + normal Hello/Welcome. Pair accept needs this to persist only a handshake-verified route. Do not infer verification from every ticket URL and do not expose a direct IP.
- Share one `PairHandshakeAdmission` between inbound and outbound pair paths: global semaphore `max_pairing_handshakes` and per-Endpoint count `max_pairing_per_endpoint`. The current `PreAuthLimits` at `network.rs:722-789` applies only inbound; a second unrelated outbound semaphore would violate the global bound.
- Pair dial uses the existing address lookup/connect budgets inside the one 15-second pairing absolute deadline. It must not call `insert_relay`, `into_0rtt`, or mutate the configured Relay map.

#### PairingService / PairingManager support

Add:

```rust
PairingService::create_until(request, deadline) -> PairOfferCreated
PairingService::accept_until(request, deadline) -> PairAcceptResult
PairingService::accept_pair_connection(connection) -> Result<(), DaemonError>
PairingService::shutdown_until(deadline) -> Result<(), DaemonError>
```

The manager itself does not need a second offer state machine. It may need only test/fault hooks around the adapter (drop `PairAccepted`, fail after durable authorize), not production-visible state.

Add a bounded async `AcceptOperationRegistry` keyed by `EphemeralOperationId`. Same ID + same server-computed fingerprint joins the running task or replays its terminal result; same ID + different fingerprint returns `PairOutcomeUnknown`. The operation is spawned once and continues after the local socket waiter times out. A new operation ID with the same ticket is allowed to perform normal confirmation repair after a prior outcome-unknown result.

#### Core/proto fingerprint and redaction

The wire currently accepts an arbitrary 32-byte fingerprint (`crates/proto/src/lib.rs:1294-1308`). That value cannot be trusted as the semantic fingerprint: a caller could reuse an operation ID and deliberately send the same forged digest for different arguments.

Add server/client-shared, domain-separated helpers such as:

```rust
PairFingerprint::for_create(effective_ttl_seconds)
PairFingerprint::for_accept(ticket_text_bytes, explicit_alias)
```

The server recomputes and compares it before allocating a cell; the local client uses the same helper. Hash incrementally so the ticket is not copied into a retained fingerprint buffer.

Also replace derived proof-bearing Debug implementations with redacted Debug. `PairProof` currently derives `Debug` at `crates/core/src/pairing.rs:633`, and `PairAccepted` derives `Debug` at `:659`; both can print proof bytes if a future error/log uses `?value`, contrary to the task's no-proof logging contract.

#### StoreActor

Existing calls are sufficient for the normal successful path:

- host authorize: `StoreHandle::authorize` (`store.rs:747`);
- reconciliation: `StoreHandle::authorization_snapshot` (`:784`);
- controller commit: `StoreHandle::upsert_known_device` (`:847`).

Two small safety adjustments are needed:

1. `wait_for_store_response` must classify response-channel disconnect after `COMMAND_STARTED` as `OperationOutcomeUnknown`, not plain `StoreUnavailable` (`store.rs:1323-1349`). Pairing may roll back only a result known not to have committed.
2. Add a pair-confirmation upsert variant whose `None` route preserves an existing verified route. The current upsert replaces route columns with NULL (`store.rs:570-602`), so a later direct-only repair could erase a previously verified relay cache. Suggested API:

```rust
StoreHandle::confirm_known_device(
    endpoint_id, alias, remote_name, verified_route: Option<RelayRouteCache>, deadline
)
```

For a new row, `None` is allowed in a direct-only test; for an existing row, it preserves route fields. An explicit verified relay replaces them atomically with alias/name in the same SQLite transaction.

### Host pair-ALPN handshake and durable ordering

Use one total absolute deadline `start + pairing_total_deadline` (15 seconds), never reset it. The first PairBegin frame additionally uses `min(start + first_frame_deadline, total)` (5 seconds). Each frame body is decoded with the existing `FrameDecoder::with_maximum_body_bytes(max_pair_hello_frame_bytes)` (16 KiB).

Retain one decoder for the receive half across PairBegin and PairProof so coalesced/partial bytes are not discarded. Add a small `PairFrameReader` wrapper around the existing decoder, not another frame codec. It should retain queued decoded frames and count every raw byte read. Count every encoded outbound frame before writing. Feed those counts to `PairHandshakeBudget`; reject before total traffic exceeds 64 KiB. Require the expected kind, `request_id == 0`, and `deadline_ms == 0` for every pair protocol frame.

Exact host sequence:

1. Await full TLS and take controller identity only from `connection.remote_id()`; reject self-pairing. Never accept a controller ID from protobuf.
2. Read/decode `PairBegin`; call `manager.prepare_challenge_until(controller_id, &begin, total_deadline)`.
3. Encode/account/write `PairChallenge`.
4. Read/decode/account `PairProof`; call `manager.try_consume_until(prepared, &proof, total_deadline)`. Invalid proof leaves Ready.
5. Acquire `AuthorizationRegistry::authorize_guard(controller_id)` under `timeout_at(total_deadline, ...)`. Timeout before durable work is safe to roll back.
6. While holding that write guard, call `store.run_blocking_until(total_deadline, |store, d| store.authorize(controller_id, controller_name, now_unix, d))`.
7. Exact StoreActor success: call `manager.commit(consumption, generation, diagnostic_version)`, then `guard.publish(Authorized, generation)`, then encode/account/write `PairAccepted`.
8. Exact transaction/pre-start failure: call `manager.rollback(consumption)` and send only generic pairing rejection.
9. Ambiguous started outcome: do not roll back. While still holding the auth write guard, enqueue `authorization_snapshot` after the authorize command and compare with the guard's pre-write snapshot. A strictly advanced Authorized generation proves the earlier command committed; finish manager commit/publish and let the controller recover by PairAccepted or normal confirmation. If reconciliation cannot prove it, drop `PairConsumption` so the offer stays Consuming and fail closed.
10. If SQLite committed but manager confirmation construction unexpectedly fails, publish the durable snapshot so normal confirmation reflects SQLite truth, leave the offer Consuming, and return a generic peer error. Never reopen it.

Only the peer-safe projection from `PairingError::peer_error` may cross the pair connection. State-specific errors (missing/expired/consumed/wrong proof) stay indistinguishable. Close/reset just this pair connection; do not call broker primary close, Session detach, or daemon stop.

### Controller local pair-accept and normal confirmation

Exact controller sequence:

1. Move `LocalPairAcceptRequest.ticket` immediately into `Zeroizing<String>`. Enforce 16 KiB before decode, call `decode_pair_ticket`, validate wall-clock expiry, reject self-pairing, validate ticket host as an Iroh EndpointId, and validate/choose alias.
2. Reserve the chosen alias through the lifecycle-shared `DeviceDirectory::reserve_selected_alias` (`device_directory.rs:174`) before any network work; hold the RAII reservation through the local SQLite commit.
3. Start/join the bounded accept operation cell before network work. The spawned operation, not the local waiter, owns the zeroizing ticket/secret.
4. Acquire shared pair admission and call `connect_pair_transient(ticket.host_device_id(), ticket.relay_hints(), total_deadline)`.
5. Generate the controller nonce with SystemRandom, send PairBegin, validate PairChallenge with `controller_transcript`, derive the offer key in zeroizing ownership, send the exact controller proof, and validate PairAccepted's host confirmation in constant time.
6. Whether PairAccepted arrives, is generically rejected, or is lost after PairProof, attempt normal `zterm/1` confirmation using `broker.demand_transient(...)` and `demand.confirm_authorization(total_deadline)`. A generic rejection alone never reopens or diagnoses the remote offer.
7. Only normal Welcome proving that this controller is currently authorized permits local persistence. Call `confirm_known_device(host_id, reserved_alias, ticket.host_name, verified_relay, deadline)` and then return the directional `DeviceSummary` (B knows A; B does not authorize A).
8. If PairAccepted was valid, normal confirmation generation may equal or exceed its generation; a later reauthorization of the same controller is safe. It must never be zero.
9. If normal confirmation or local commit remains ambiguous after proof may have been accepted, return `PairOutcomeUnknown`. Do not request remote rollback/RevokeSelf. A later accept with the same ticket and a fresh operation ID may skip/retry the failed pair attempt, use normal confirmation, and repair the known-device row.
10. If no proof could have been committed and normal confirmation also fails, return the appropriate local address/invalid-ticket category without changing known devices. Host-side generic rejection must not be translated into a claim that the offer was definitely missing versus consumed.

This sequence preserves one-way authorization: host A writes only `device_auth(B)` and publishes A's registry; controller B writes only `known_devices(A)`. A has no outbound known row for B, and B's receiver-side authorization check still rejects reverse service streams (`connection_broker.rs:1697-1707`).

### Local IPC and secret-buffer handling

- Add `pairing: Option<PairingService>` to `DaemonService`, route kinds 12/14 through the async-native branch, and leave device/store synchronous portions on `spawn_blocking`.
- Add a hidden `LocalPairingClient` next to `LocalDeviceClient`. It generates a 128-bit operation ID with `ring::SystemRandom`, computes the server-checkable fingerprint, encodes once, and uses one byte-identical ambiguous transport retry under a 15-second request deadline. It is not wired into clap/public CLI.
- Pair create maps TTL zero to `DEFAULT_PAIR_TTL_SECONDS`; nonzero TTL remains 60-3600 seconds. It requires `NetworkState::Online`, `endpoint_bound`, and a nonempty current home relay before `PairOfferRequest`; merely having a stale `home_relay` string in Degraded state is insufficient.
- Make sensitive request/reply byte ownership zeroizing. Moving the frame removed the generic clone, but `DecodedFrame.payload`, the temporary `WireFrame` body, `ServiceReply.bytes`, the client encoded request, and decoded ticket/proof strings currently remain ordinary Vec/String allocations. At minimum zeroize the decoder body after protobuf decode, wrap pair payload/request/reply bytes, explicitly zeroize generated proto secret/proof fields after encoding, and use custom redacted errors. Reuse `FrameDecoder`; do not add a pair-specific binary framing implementation.

### Tests

#### Pure and macOS-safe (must not bind Endpoint)

- Keep and extend `pairing_manager.rs`: exact StoreActor-success/rollback/ambiguous-reconciliation decision tests, manager commit invariant failure remains fail-closed, server-recomputed fingerprint mismatch, and proof-bearing Debug redaction.
- Add adapter tests with fake pair transport/normal confirmer and task-private SQLite: invalid proof leaves Ready; exact DB failure rolls back; started/unknown DB outcome reconciles or stays Consuming; generation exhaustion never consumes; PairAccepted-drop followed by normal confirmation commits exactly one known row.
- Extend `local_device_ipc` or add `local_pair_ipc` using injected `PairingService` transport seams: kinds 12-15, strict EOF, 16 KiB ticket, 15-second deadline, create/accept response loss replay, alias reservation conflict, no public CLI surface, and sentinel absence from errors/status/SQLite/captured tracing.
- Pure admission tests prove 8 global/1 Endpoint across inbound and outbound logical handshakes without sockets.
- Lifecycle injected-bind-failure tests keep pair create at `AddressUnavailable`, local readiness/session/device calls responsive, and shutdown bounded; these tests must keep `usize::MAX` pre-bind failure injection.

#### Linux real-Iroh (loopback-only)

- `pairing_protocol`: two task-private endpoints, pair A<-B, normal confirmation, only `device_auth_A(B)` plus `known_devices_B(A)`, reverse stream rejected, pair connection absent from primary registry.
- Concurrent consumers with barriers: one CAS/Store authorize; loser gets generic rejection; consumed replay never authorizes a second EndpointId.
- Drop PairAccepted after host commit and prove normal confirmation repairs B exactly once. Then use an independent reverse ticket and prove independent generations/revocation.
- Verify expired/tampered/wrong host/wrong proof, full 5s/15s/64KiB limits, pair global/per-peer overload, and shutdown cancellation.
- Keep all real-Iroh tests `#[cfg_attr(target_os = "macos", ignore = "...")]` and retain the fixture's runtime macOS guard. Do not run them locally on macOS. Relay/cross-profile evidence remains an explicit Linux/manual environment gate, not an ordinary test.

### Security review points

- **0-RTT:** use only awaited `Incoming`/`Accepting` and `Endpoint::connect`; forbid `into_0rtt` by source test.
- **Unknown controller ID:** bind the transcript and StoreActor key solely to TLS `remote_id`; PairBegin has no endpoint-ID authority. Controller validates TLS host ID against the ticket before proof.
- **PairAccepted loss:** any path after sending PairProof is potentially committed. Never roll back from a transport timeout; normal confirmation is the recovery oracle.
- **One-way authorization:** host authorize and controller known-device writes are separate; pair QUIC is never promoted; every reverse normal stream still checks the receiver's local registry.
- **Ambiguous SQLite outcome:** rollback is safe only when the command is known not to have started or returned an exact failed transaction result. A started/disconnected command must reconcile or leave Consuming.
- **Fingerprint authority:** recompute on the server; never let a caller-supplied digest define semantic equality.
- **Proof leakage:** custom Debug and zeroizing adapter buffers are required even if current call sites do not log the values.

## Files Found

- `crates/daemon/src/pairing.rs` — complete in-memory offer/proof/CAS foundation; no Iroh/StoreActor orchestration yet.
- `crates/daemon/src/network.rs` — full-TLS ALPN router and inbound limits; pair branch is the current placeholder.
- `crates/daemon/src/connection_broker.rs` — sole Endpoint, route resolver, normal Hello/Welcome, transient demand, and one-way receiver checks.
- `crates/daemon/src/service.rs` — async daemon dispatch and completed device-management/revoke composition.
- `crates/daemon/src/local_ipc.rs` — strict same-UID unary framing, moved sensitive frame, generic retry client, and device client.
- `crates/daemon/src/lifecycle.rs` — production construction site with one shared StoreActor/auth registry/device directory/network owner.
- `crates/daemon/src/store.rs` — bounded actor commands, generation transactions, known-device upsert, and started/outcome-unknown gate.
- `crates/daemon/src/device_directory.rs` — shared alias selection/reservation owner.
- `crates/core/src/pairing.rs` — canonical ticket/transcript HMAC and handshake byte budget.
- `crates/proto/src/lib.rs` and `proto/zterm/v1/pairing.proto` — ticket/pair adapters and kinds 12-15/100-103.

## External References

- The repository pins Iroh exactly at 1.0.3; current integration uses its awaited full-TLS `Endpoint::connect`/`Incoming` path and must remain on that API for this task.
- `.trellis/spec/backend/relay-deployment.md` is the active infrastructure contract: official and self-hosted maps remain separate, remote ticket routes are dial candidates only, and the Endpoint accepts both product ALPNs.
- No public network or Endpoint bind was used for this research.

## Related Specs

- `.trellis/spec/backend/core-wire-domain.md` — one wire-kind registry, one FrameDecoder, frame/control bounds, and transport-neutral core.
- `.trellis/spec/backend/local-daemon-ipc.md` — same-UID gate, strict unary EOF, one absolute deadline, async/blocking separation, and local readiness independent of network.
- `.trellis/spec/backend/relay-deployment.md` — exact Iroh profile, pair ALPN, relay-only publication, and no configured-map mutation.
- `.trellis/tasks/08-22-transport-auth/design.md:319-458` — pairing transcript/state machine, local accept recovery, and store/auth ordering.
- `.trellis/tasks/08-22-transport-auth/implement.md:294-333` — Step 7 work/tests/gates.

## Caveats / Not Found

- There is no current pair-ALPN host adapter, outbound pair dial, LocalPairingClient, pair service injection, accept replay cell, or normal confirmation method.
- There is no server-owned semantic fingerprint constructor; raw wire fingerprints are only length-checked.
- The current StoreActor disconnect category is not sufficient by itself to decide rollback after a started authorize command.
- The current normal broker persists a verified relay asynchronously only for an existing known row, so pair acceptance cannot rely on that background task to create its address-book entry.
- Pair protocol integration must account raw framed bytes with a persistent decoder; the broker's one-frame helper returns no byte count and is not sufficient unchanged for the two-message receive sequence.
- Real-Iroh evidence cannot be executed safely on this macOS workstation because it may trigger the application firewall prompt. Linux CI/manual evidence remains required.
