# Iroh Transport, Pairing, and Device Authorization Contract

## 1. Scope / Trigger

Apply this contract when changing the daemon-owned Iroh endpoint, ALPN routing,
relay route selection, connection singleflight, pairing, inbound authorization,
device IPC, or remote-principal revocation. These paths cross `zterm-core`,
`zterm-proto`, `zterm-daemon`, SQLite, local IPC, and the retained Session
service, so their order and ownership are product contracts rather than adapter
details.

This contract implements the M5-M6 foundation. It does not expose the M7 remote
Session RPC adapter or M8 public pair/device CLI.

## 2. Signatures

```rust
pub const ZTERM_ALPN: &[u8] = b"zterm/1";
pub const ZTERM_PAIR_ALPN: &[u8] = b"zterm-pair/1";

NetworkStartup::prepare(
    identity: DeviceIdentity,
    profile: InfrastructureProfile,
    connection_identity: ConnectionIdentity,
    store: StoreHandle,
    authorization: AuthorizationRegistry,
    limits: TransportLimits,
) -> Result<(NetworkStartup, NetworkHandle), DaemonError>

RouteResolver::candidates(
    &self,
    endpoint: &Endpoint,
    remote: DeviceId,
    transient_ticket_routes: &[RelayHint],
    deadline: Instant,
) -> Result<Vec<RouteCandidate>, DaemonError>

ConnectionBroker::demand(remote: DeviceId, deadline: Instant)
    -> Result<ConnectionDemand, DaemonError>
ConnectionDemand::confirm_authorization(deadline: Instant)
    -> Result<AuthorizationConfirmation, DaemonError>
ConnectionDemand::open_bi(purpose: StreamPurpose, deadline: Instant)
    -> Result<AuthenticatedBiStream, DaemonError>

PairingService::create_until(input: LocalPairCreateInput, deadline: Instant)
    -> Result<PairOfferCreated, DaemonError>
PairingService::accept_until(input: LocalPairAcceptInput, deadline: Instant)
    -> Result<PairAcceptResult, DaemonError>
PairingService::accept_pair_connection(connection: PairConnection, deadline: Instant)
    -> Result<(), DaemonError>

AuthorizationRegistry::acquire_commit(device_id, expected_generation)
    -> Result<AuthorizedCommitContext, DaemonError>
AuthorizationRegistry::authorize_guard(device_id)
    -> Result<AuthorizationWriteGuard, DaemonError>
AuthorizationRegistry::revoke_guard(device_id)
    -> Result<AuthorizationWriteGuard, DaemonError>

LocalDeviceClient::new(socket: impl Into<PathBuf>) -> LocalDeviceClient
LocalDeviceClient::{list, rename, revoke}(...) // Unix: IPC; non-Unix: UnsupportedPlatform

// Linux-only lib-test acceptance target; not a production API.
pairing_service::multiprocess_test::
    two_process_production_pairing_service_is_directional_and_reuses_one_endpoint
```

Wire kinds 12-21 are same-UID local pair/device requests and responses. Pairing
uses kinds 100-103 on `zterm-pair/1`; normal Hello/Welcome use 104-105 on
`zterm/1`. `RelayRouteCacheV1` persists relay URLs only. `DeviceSummary` keeps
outbound-known and inbound-authorization directions explicit.

## 3. Contracts

### Endpoint and lifecycle ownership

- One production daemon owns one long-term `DeviceIdentity`, one `Endpoint`, one
  `ConnectionBroker`, one `StoreActor`, one `AuthorizationRegistry`, one
  `DeviceDirectory`, and one `PairingService`. Pairing reuses that endpoint;
  it never creates a second endpoint or inserts a relay into the configured
  profile.
- Local IPC readiness does not wait for endpoint bind, DNS, relay, or Internet.
  Bind failure publishes a truthful degraded observation and retries with
  bounded jitter while local Store/Session/socket owners remain available.
- Product startup uses `run_daemon`; the doc-hidden local-only test entry must
  never prepare or bind Iroh. Shutdown quiesces pairing and broker work before
  closing the endpoint, while retained Session ownership remains authoritative.
- Incoming ALPN classification is exact. Unknown ALPNs are dropped; normal and
  pair pre-auth budgets are separate, with one shared outer connection bound.
  Completed Iroh TLS authentication is required; no 0-RTT application path is
  allowed.

### Routes, connections, and streams

- Route order is fresh signed lookup, verified SQLite cache, then transient
  ticket route. `max_relay_hints` bounds each source independently. The merged
  fallback sequence must not apply the same value as a global cap: a full fresh
  set must still leave cache/ticket candidates available after connection
  failures. Duplicate URLs are removed while preserving source order.
- Every candidate `EndpointAddr` contains only the target endpoint ID and one
  relay URL. Direct addresses may be observed from a live Iroh path but are
  never persisted, returned as a relay hint, or added to the configured relay
  map. Only a successful TLS plus application handshake verifies a cache route.
- A per-device `PeerSlot` owns demand count, a single dial worker, provisional
  candidates, deterministic primary selection, transient-route leases, and
  terminal/retryable error state. Dropping demand or stream permits releases
  exact capacity; duplicate/path loss never ends a Session.
- Apply normal QUIC limits immediately after connect/accept and before
  Hello/Welcome: advertise `max_bi_streams_per_connection` and zero
  unidirectional streams. Pair connections advertise one bidirectional and zero
  unidirectional streams. Application handler and open-request queues have
  independent per-connection/global bounds.
- Inbound normal authorization is checked before reading Hello. Candidate
  registration is followed by an exact-generation recheck so a queued revoke
  cannot publish stale access. Each business side effect holds an
  `AuthorizedCommitContext` read permit through its blocking commit.

### Pairing and authorization

- Ticket text is `zterm-pair-v1:` plus base64url-no-pad protobuf, bounded to
  16 KiB before decode. The canonical ticket/transcript builders live in core;
  protobuf encoding is never an HMAC input. Tickets contain public host fields,
  at most four HTTPS relay hints, a 16-byte offer ID, a 32-byte bearer secret,
  and an absolute expiry.
- Pair-create and pair-accept use random 16-byte ephemeral operation IDs plus a
  fingerprint of every semantic argument. Same ID and fingerprint join/replay
  exact bytes; a mismatch is outcome unknown and never re-executes.
- Offer state is `Ready -> Consuming -> Consumed` or `Expired`. Controller proof
  is verified before CAS. A known pre-commit failure rolls `Consuming` back to
  `Ready`; a successful SQLite authorization commits and publishes before
  `PairAccepted`. A dropped or otherwise ambiguous consumption remains
  `Consuming` unless durable generation evidence proves the exact commit.
- Pair acceptance reserves its alias before transport, validates ticket host
  against TLS identity, performs the pair exchange, then uses normal
  Hello/Welcome as receiver-owned authorization confirmation. The controller
  persists only its outbound known-device row and a handshake-verified relay;
  pairing never invents reverse authorization.
- Authorization is directional. `device_auth` controls who may operate this
  host; `known_devices` controls whom this host can dial. Generation increments
  are checked against SQLite's signed `i64` ceiling and never saturate or wrap.
- Revoke order is exclusive authorization writer, SQLite commit, in-memory/watch
  publication, broker close for that remote, remote-principal detach from every
  Session, then local response. A database failure changes none of the later
  owners. Detach removes only the matching remote attachments/controller lease;
  it does not close the Session, PTY, or another principal.

### Secret and user-surface boundaries

- Ticket, secret, nonce, proof, confirmation, decoded payload, and encoded local
  request owners are zeroized at their narrowest owner. Their `Debug` output is
  redacted; prost generation uses exact `skip_debug` entries for sensitive
  messages and `WireFrame`. Errors/status/tracing/SQLite never contain bearer
  or proof bytes.
- Pair/device local IPC remains behind the same-UID gate, strict unary EOF, one
  shared frame decoder, and byte-identical ambiguous retry. The hidden clients
  do not spawn the daemon or bind an endpoint. Public clap still exposes no
  pair/device/connect/session commands or state/identity/socket override.

### Platform compilation boundaries

- A cross-platform hidden client may keep a stable constructor and typed
  non-Unix `UnsupportedPlatform` methods, but its Unix socket-backed private
  fields exist only under `#[cfg(unix)]`. Non-Unix construction consumes the
  socket argument and produces a stateless value; it must not retain an unused
  `LocalClient` merely to keep the struct shape identical.
- When an actor query has only a Unix consumer, gate the whole private chain
  together: service method, command variant, dispatch match arm, and helper.
  Gating only the caller leaves dead private code in the Windows shared build.
  Do not hide this drift with `allow(dead_code)` or `expect(dead_code)`.
- Hosted Windows Clippy for workspace `--lib --bins --all-features` with
  `-D warnings` is the authoritative boundary. A macOS cross-check may stop in
  native C/assembly dependencies before project code and does not replace the
  hosted result.

### Linux multi-process pairing evidence

- The real-Iroh pairing acceptance target self-spawns exact ignored host and
  controller helper tests as distinct OS processes. Each helper owns its own
  task-private `UserPaths`, identity, StoreActor, authorization/device owners,
  one loopback-only Endpoint, broker, and production
  `PairingService`/`BrokerPairTransport` composition.
- Test environment keys carry only the helper role and the task-private Unix
  control-socket path. The bearer ticket travels only in a bounded control
  packet over that socket; it must never enter argv, environment values, files,
  snapshots, panic text, stdout, or stderr. The temporary control root is mode
  `0700`, each socket is mode `0600`, and decoded ticket-bearing packet owners
  are dropped or zeroized immediately after transfer.
- Every control read/write and child exit has a deadline. Helper stdout/stderr
  go to the null device rather than a pipe; on timeout or parent unwind, the
  parent kills and boundedly polls/reaps each child. Never combine piped output
  with an unbounded `wait`/`wait_with_output` in this bearer-adjacent gate.
- The parent is ignored on every non-Linux platform, and the parent plus both
  helper entrypoints assert Linux before constructing or binding an Endpoint.
  macOS development runs may compile/list this target but must not execute it.
- This target is evidence for two-process production `PairingService`,
  `BrokerPairTransport`, pair/normal ALPN behavior, directional persistence,
  and one-Endpoint reuse. Its private test-only ALPN router is not evidence for
  `run_daemon`, `NetworkStartup`, the future public CLI, or full daemon
  lifecycle recovery.

## 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| local identity, broker identity, or TLS remote ID differs | `identity_state_mismatch` or generic remote `unauthorized`; no state mutation |
| unknown/revoked inbound normal peer | generic `unauthorized` before Hello payload decode |
| missing/invalid/expired route set or lookup deadline | `address_unavailable` / `deadline_exceeded`; try the next ordered source when safe |
| count, byte, queue, connection, stream, or pairing permit exhausted | `resource_exhausted`; no unbounded task/byte queue |
| unknown ALPN, wire major, or handshake kind | connection-local typed protocol failure |
| invalid/tampered/expired/consumed pairing ticket or proof | typed local pair error, generic peer pair error, no authorization commit |
| same operation ID with a different fingerprint | outcome unknown; do not replay or execute |
| authorization generation at `i64::MAX` | explicit exhaustion; no row, registry, or offer-state wrap |
| StoreActor disconnect before a mutation starts | exact store/deadline error; rollback is permitted |
| StoreActor response loss after mutation starts | `operation_outcome_unknown`; reconcile durable state before deciding rollback |
| revoke SQLite failure | return error; registry, connections, attachments, and generation unchanged |
| network shutdown while pairing/dial/stream waits | `cancelled`/`transport_unavailable`; release every RAII permit |
| multi-process gate on a non-Linux host | parent ignored; every callable helper fails before Endpoint bind |
| control packet has wrong kind, zero/oversize length, timeout, or trailing bytes | fail the test locally; do not decode/use a ticket or continue the handshake |
| pairing helper misses the shared exit deadline | kill, boundedly reap, and fail without emitting child output |
| Unix-only private field or actor-query chain in a shared Windows module | gate every private owner/variant/arm/helper together; shared Clippy must have zero dead code |
| non-Unix caller constructs or invokes `LocalDeviceClient` | construction succeeds without Unix state; operation returns typed `UnsupportedPlatform` |

## 5. Good / Base / Bad Cases

- **Good:** four fresh candidates fail, then verified cache and transient ticket
  candidates remain available in that order; a successful normal handshake is
  the only route-persistence point.
- **Good:** proof succeeds, the host durably authorizes the controller, a lost
  `PairAccepted` is reconciled, and normal confirmation completes the
  controller's outbound known-device row exactly once.
- **Base:** endpoint bind is degraded while same-UID status, StoreActor, and
  retained Sessions remain responsive.
- **Base:** macOS/other non-Linux hosts compile or list the multi-process target,
  but the target remains ignored and its helper guard precedes every bind.
- **Base:** Windows compiles the shared hidden device-client surface without a
  Unix socket field; invoking an operation returns `UnsupportedPlatform`.
- **Bad:** cap the combined route list at four, accept unidirectional QUIC
  streams that no actor consumes, persist a direct address, roll back an
  outcome-unknown authorization, detach a PTY because one device was revoked,
  place a bearer ticket in child argv/env/output, or leave a Unix-only actor
  command compiled and unused on Windows.

## 6. Tests Required

- Core/proto: `pairing_vectors` and proto compatibility cover canonical bytes,
  proofs, fingerprints, ticket text, limits, directional device DTOs, kind
  registry, unknown capability bits, and sensitive Debug sentinels.
- Pure daemon: broker/route/network/pair manager/service tests cover admission,
  deterministic candidates, per-source fallback, early bi/zero-uni limits,
  deadlines, cancellation, offer state, ambiguity reconciliation, alias-before-
  transport, and secret redaction without opening a socket.
- Unix IPC: `local_pair_ipc`, `local_device_ipc`, `revoke_races`, `local_ipc`,
  and `local_session_ipc` cover strict EOF, exact retry, ordered revoke, matching-
  principal detach, response loss, and listener/session independence.
- Named transport gates: `duplicate_connection`, `stream_limits`,
  `authorization`, `path_migration`, and `network_lifecycle` must be
  deterministic and socket-free where possible.
- Real Iroh targets are compiled on developer macOS but not executed there.
  Linux CI owns execution of `connection_broker` and `two_daemon_transport`,
  and this exact two-process production pairing target:

  ```sh
  cargo test -p zterm-daemon --lib --all-features \
    pairing_service::multiprocess_test::two_process_production_pairing_service_is_directional_and_reuses_one_endpoint -- --exact
  ```

  Its assertions include distinct process IDs/owners, ticket-only private
  control transport, directional durable/registry state, pair-to-normal
  confirmation, one normal primary with zero business streams, one Endpoint
  identity/socket per child, and no direct-route persistence. Linux also owns
  the disposable self-hosted relay/static/public handshake gates. Hosted
  Windows owns shared core/proto/daemon compile evidence, including:

  ```sh
  cargo clippy --workspace --lib --bins --all-features -- -D warnings
  cargo test -p zterm-core -p zterm-proto -p zterm-platform -p zterm-daemon \
    --lib --all-features
  ```
- Every change runs workspace fmt, check, Clippy with `-D warnings`, tests,
  docs, dependency/source/version/secret policy, task validation, and
  `git diff --check`.

## 7. Wrong vs Correct

### Wrong

```rust
for route in fresh.into_iter().chain(cache).chain(ticket).take(max_relay_hints) {
    dial(route).await?;
}
```

This turns a per-source storage bound into a merged bound, so a full but stale
fresh result silently disables the designed cache/ticket fallback.

### Correct

```rust
for source in [fresh, cache, ticket] {
    for route in source.into_iter().take(max_relay_hints) {
        if seen.insert(route.clone()) {
            ordered.push(route);
        }
    }
}
```

Each independently validated source stays bounded, deduplication preserves the
fallback order, and later sources remain reachable after earlier dial failures.

For the multi-process acceptance harness, piping bearer-adjacent child output
and waiting without a deadline is likewise wrong:

```rust
let child = Command::new(current_exe).stdout(Stdio::piped()).spawn()?;
let output = child.wait_with_output()?; // pipe backpressure and wait are unbounded
```

Use null output plus one parent-owned deadline and bounded kill/reap instead:

```rust
let child = Command::new(current_exe)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()?;
let status = finish_or_kill_until(child, shared_deadline)?;
```

This prevents output backpressure from hanging Linux CI and prevents sensitive
child diagnostics from being copied into the parent test result.

For shared modules, leaving Unix-backed private state or commands compiled on
Windows is also wrong:

```rust
pub struct LocalDeviceClient { client: LocalClient }
enum SessionCommand { CountRemoteAttachments { /* ... */ } }
```

Keep the public unsupported surface while gating the complete private chain:

```rust
pub struct LocalDeviceClient {
    #[cfg(unix)]
    client: LocalClient,
}

enum SessionCommand {
    #[cfg(unix)]
    CountRemoteAttachments { /* ... */ },
}
```

The corresponding service method, dispatch arm, and helper carry the same
`cfg(unix)` boundary, so every compiled target has a real consumer.
