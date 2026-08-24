# Core and Wire Domain Contract

## 1. Scope / Trigger

Apply this contract when changing shared identifiers, terminal revisions,
capabilities, resource defaults, operation replay, protobuf messages, wire
kinds, or frame encoding. `zterm-core` owns product domain values and
`zterm-proto` owns their wire representation and validation.

## 2. Signatures

```rust
pub struct OperationLease {
    pub daemon_incarnation: DaemonIncarnation, // exactly 16 bytes
    pub ordinal: u64,                          // non-zero, daemon-issued
}

pub struct OperationId {
    pub lease: OperationLease,
    pub sequence: u64, // non-zero, checked before wrap
}

OperationWindow::new(lease: OperationLease, capacity: usize)
    -> Result<OperationWindow<R>, OperationWindowError>
OperationWindow::execute(id: OperationId, operation: impl FnOnce() -> R)
    -> Result<OperationOutcome<R>, OperationWindowError>
```

The mutation wire boundary is
`SessionOperationLeaseRequest -> SessionOperationLeaseResponse { lease }`, then
one request carrying `OperationId { lease_ordinal = 1, sequence = 2,
daemon_incarnation = 3 }`. `WireKind` values are registered once in
`crates/proto/src/lib.rs`; session lease kinds are 207/208 and the product ALPN
is `zterm/1`.

## 3. Contracts

- `DeviceId` is 32 bytes; `SessionId` and `AttachmentId` are 16 bytes.
- `Revision` is the only public terminal revision type. It is monotonic and
  checked before mutation.
- `SessionName` is the only validator for exact case-sensitive session names;
  `SessionSelector` resolves either a validated name or a 16-byte ID, and
  `SessionEndReason` distinguishes natural exit, explicit close, daemon stop,
  and driver failure without retaining terminal content.
- `AttachmentPrincipal` distinguishes an authenticated remote endpoint from a
  same-UID local view. A local principal is created only after the platform
  peer-credential gate succeeds.
- Capability values retain unknown bits. Optional future capabilities never
  become prerequisites for ordinary terminal service.
- `DomainErrorKind::code` and `from_code` are the single stable error-category
  bridge used by wire and JSON projections; adapters do not invent aliases.
- `OperationWindow` is fixed to one daemon-issued `OperationLease` and retains
  exact results in a bounded non-zero sequence window. A retained duplicate
  replays its result; a lease mismatch, zero sequence, or evicted sequence
  returns outcome unknown and is never run. Sequence exhaustion is reported
  before wraparound.
- An `OperationLease` contains a random 16-byte daemon incarnation and a
  daemon-monotonic non-zero ordinal for one stable principal/auth generation.
  It is allocated by the daemon, never invented from wall clock, process ID, or
  client randomness. Incarnation mismatch is rejected before inspecting or
  changing ordinal/floor state; a restart therefore makes every old lease
  outcome unknown without executing it.
- Issued ordinals live in a bounded registry. Lost allocation responses may
  leave empty leases, so they participate in the same completed-prefix
  retirement as used leases. Retired, missing, invented, or high ordinals
  return outcome unknown and are never recreated. Ordinal exhaustion is
  explicit and cannot wrap.
- M2 owns and tests the replay state machine; M4 integrates it around stateful
  `SessionService` create/rename/close/takeover commits. Successful and typed
  error results are replayed exactly. Each result is also bound to a fingerprint
  of every semantic mutation argument, so reusing an ID for another payload is
  outcome unknown. Local replay keys use the stable daemon device identity,
  authorization generation zero, and issued lease ordinal, never the per-socket
  view ID. M4 local IPC is admitted only after the same-UID peer gate;
  authorization generation zero therefore means the current daemon owner's
  trust boundary, not a remote ACL.
- Readiness, status, and list allocate no lease and write no replay state. A
  logical local client lazily caches a lease before its first mutation. A local
  target may retry its same-UID mutation once with byte-identical bytes and
  operation ID. For a remote target, the daemon-owned `RemoteUnaryClient` is the
  sole mutation retry owner: the outer same-UID envelope is sent once after any
  bytes are written, while the daemon may open at most two Iroh service streams
  under one deadline with the same target, request bytes, lease, and operation
  ID. `SessionOperationLeaseRequest` is stateful control rather than read-only:
  after a post-write transport/protocol ambiguity it returns that typed failure
  and opens no second remote stream, so one logical allocation cannot silently
  issue two leases. There is no disk-backed lease or automatic fresh-process
  recovery; only an API which explicitly exports an opaque retry token may
  continue an ambiguous operation in another client object.
- `proto/zterm/v1/*.proto` is the wire source of truth. One numeric kind
  registry and one decoder own all message dispatch.
- Frames are `varint length + WireFrame`, capped at 8 MiB before body
  allocation. Control payloads are capped at 1 MiB before concrete-message
  decoding. Unknown protobuf fields are compatible; unknown kind and wire
  major are explicit errors.
- The normal product ALPN is `zterm/1`; short-lived pairing uses
  `zterm-pair/1`. Core defines transport-neutral values and never binds an Iroh
  endpoint; daemon ownership and authorization sequencing follow the
  [transport/auth contract](./transport-auth.md).

## 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| identifier or incarnation has the wrong byte length | reject during domain conversion before dispatch |
| replay capacity is zero | `OperationWindowError::InvalidCapacity` |
| lease incarnation/ordinal is missing, invented, retired, or from another daemon | outcome unknown; do not execute |
| operation sequence is zero, evicted, or would wrap | outcome unknown/exhaustion; do not execute |
| same operation ID carries a different semantic payload | outcome unknown; do not replay or execute |
| frame prefix is malformed/non-canonical or body is truncated | protocol error scoped to that connection |
| frame exceeds 8 MiB or control payload exceeds 1 MiB | reject before allocating/decoding the concrete body |
| wire major or kind is unsupported | explicit protocol/service error; listener remains healthy |

## 5. Good / Base / Bad Cases

- **Good:** obtain a daemon lease lazily, encode one mutation once, and let
  exactly one transport layer reuse its exact bytes and operation ID for the
  single ambiguous-transport retry.
- **Base:** readiness/status/list use no operation lease and do not alter replay
  state.
- **Bad:** derive an epoch from wall-clock time, accept a client-invented high
  ordinal, wrap a sequence, or rerun an outcome-unknown operation under a fresh
  lease.

## 6. Tests Required

- Core state-machine tests cover ID lengths, principals, unknown capability
  retention, defaults, replay, eviction, errors, fixed-lease mismatch, zero
  sequence, and no sequence wrap.
- Daemon tests cover daemon-issued monotonic leases, lost empty-lease
  retirement, admission past the active bound, in-flight retirement refusal,
  restart/incarnation mismatch, invented/high ordinal rejection, exhaustion,
  panic-safe duplicate waiters, and outcome unknown below the retired floor.
- Proto tests cover round trip, unknown fields, unknown kinds, major mismatch,
  non-canonical/malformed varints, truncated bodies, and both size limits.
- `local_ipc` proves malformed/unsupported requests terminate only their own
  unary connection and do not poison the listener. `terminal_recovery` owns
  the equivalent duplex attachment isolation evidence.

## 7. Wrong vs Correct

### Wrong

```rust
let epoch = wall_clock_nanos();
let id = OperationId::new(epoch, next_sequence());
send(encode_again(id, request)).await?;
```

This has no cross-process authority and can change request bytes between an
ambiguous attempt and its retry.

### Correct

```rust
let lease = client.daemon_issued_lease().await?;
let id = checked_operation_id(lease)?;
let encoded = encode_once(id, request)?;
send_with_one_ambiguous_retry(&encoded, deadline).await
```

The daemon validates the incarnation and issued ordinal before effects, while
the client preserves the exact retry identity and payload.

## Forbidden patterns

- Prost, SQLite, Iroh, or OS dependencies in `zterm-core`.
- Raw `u64` revisions in public terminal APIs.
- A second frame parser in the CLI or daemon.
- Re-executing an operation whose result has fallen below the replay low-water
  mark.
- Generating or accepting a client-invented lease ordinal/incarnation, wrapping
  an operation sequence, or automatically retrying an outcome-unknown mutation
  under a fresh lease.
