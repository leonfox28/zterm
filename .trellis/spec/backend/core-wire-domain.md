# Core and Wire Domain Contract

## 1. Scope / Trigger

Apply this contract when changing shared identifiers, terminal revisions and
DTOs, capabilities, resource defaults, operation replay, protobuf messages, wire
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
is `zterm/1`. Terminal history uses additive kinds 312/313 and the same-UID-only
connection-status event is kind 314. Continuous terminal viewport uses additive
kinds 315/316 and capability bit 19:

```rust
TerminalScrollMetrics { epoch, revision, offset_from_bottom,
    max_offset_from_bottom, viewport_rows }
TerminalScrollAction::{ScrollByLines(i32), ScrollToOffset(u64)}
TerminalViewportResult::{Frame, Live, HistoryChanged, HistoryGap}
Capabilities::TERMINAL_VIEWPORT == 1 << 19

TerminalHistoryWindowAnchor { epoch, revision, max_offset_from_bottom, viewport }
TerminalHistoryWindowQuery { anchor, target_offset_from_bottom,
    older_margin_rows, newer_margin_rows }
TerminalHistoryWindowResult::{Frame, HistoryChanged, HistoryGap}
Capabilities::TERMINAL_HISTORY_WINDOW == 1 << 20
MAX_HISTORY_WINDOW_ROWS == 240

ViewportCache<Row>::set_target(offset) -> ViewportCacheUpdate
ViewportCache<Row>::install_window(window)
    -> Result<ViewportCacheInstall, CachedViewportWindow<Row>>
```

## 3. Contracts

- `DeviceId` is 32 bytes; `SessionId` and `AttachmentId` are 16 bytes.
- `Revision` is the only public terminal revision type. It is monotonic and
  checked before mutation.
- `zterm-core::terminal` owns transport-neutral terminal size, screen, cell,
  style, cursor, mode, event, update, snapshot/delta, and history values.
  `zterm-proto` encodes those values. Neither crate depends on
  `zterm-terminal`, `alacritty_terminal`, or `vte`, and no upstream terminal
  type crosses a public or wire boundary.
- `SessionName` is the only validator for exact case-sensitive session names;
  `SessionSelector` resolves either a validated name or a 16-byte ID, and
  `SessionEndReason` distinguishes natural exit, explicit close, daemon stop,
  and driver failure without retaining terminal content.
- `AttachmentPrincipal` distinguishes an authenticated remote endpoint from a
  same-UID local view. A local principal is created only after the platform
  peer-credential gate succeeds.
- Capability values retain unknown bits. Optional future capabilities never
  become prerequisites for ordinary terminal service.
- `Capabilities::HISTORY_PAGING` is active only for the complete bounded
  request/page path. A history request carries attachment identity, direction,
  optional epoch/revision cursor, and a maximum row count; its correlated page
  has an explicit `Ok`, `Changed`, or `Gap` outcome. Page Debug reports only
  structural counts, never formatted terminal rows.
- `Capabilities::TERMINAL_VIEWPORT` is active only for the complete semantic
  request/frame path. Request action is exactly one signed relative or absolute
  variant. A correlated frame has an explicit Frame/Live/Changed/Gap outcome;
  Frame carries Exact/Rebased plus metrics and exactly `viewport_rows`
  independently encoded rows, Live carries valid zero-offset metrics without
  rows/disposition, and Changed/Gap carry only the current epoch/revision.
  Snapshot/delta metrics are optional for mixed versions and absent on
  alternate screen. Frame Debug reports structural counts and ANSI byte length,
  never row content.
- `Capabilities::TERMINAL_HISTORY_WINDOW` is active only for the stateless
  contiguous-window path. A request carries one immutable anchor, absolute
  target, and bounded older/newer margins. A Frame carries Exact/Rebased,
  current anchor, resolved target, signed live-top coordinate, and the exact
  request-shaped row count; Changed/Gap are content-free. Frame Debug exposes
  only coordinates, counts, and total bytes.
- `TerminalHistoryWindowQuery::response_shape` is shared by model, local IPC,
  remote bridge, and client cache validation. A response may advance but never
  predate its request; range, disposition, translated target, viewport size,
  and row count must all match that originating query.
- `ViewportCache<Row>` is renderer-neutral and contains no ANSI, async runtime,
  clock, mouse, or platform type. It retains one immutable bounded row window,
  latest monotonic anchor, desired/presented offsets, and the complete one
  outstanding query. Cache hits render locally; low-water/miss produces at
  most one request while later gestures coalesce to the latest desired target.
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
  decoding. `TerminalHistoryRequest` is control; `TerminalHistoryPage` is a
  bounded content frame under the ordinary 8 MiB limit.
  `TerminalViewportRequest` is control and `TerminalViewportFrame` is content
  under the same limits. `TerminalHistoryWindowRequest` is control and
  `TerminalHistoryWindowFrame` is content under those same 1/8 MiB limits.
  Unknown protobuf fields are compatible; unknown kind and wire major are
  explicit errors.
- Kind/capability allocation is append-only: history remains 312/313, local
  status remains 314, viewport remains 315/316, history window is 317/318,
  `AGENT_EVENTS` remains bit 18, viewport remains bit 19, and history window is
  bit 20. A peer without bit 20 receives no 317/318 frame; adapters fall back
  to negotiated 315/316 and then unchanged 312/313. No fallback advertises or
  simulates a capability the peer omitted.
- `TerminalConnectionStatusEvent` carries only attachment ID, unknown/direct/
  relay, and optional bounded integer RTT. It is never a normal-ALPN service
  kind and contains no DeviceId, address, relay URL, candidate, or ticket.
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
| history direction/outcome is unspecified, row bound is invalid, or page cursor/count disagrees | reject as malformed before projection; do not expose row content |
| viewport action/outcome/disposition is missing or inconsistent | reject as malformed before forwarding or rendering |
| viewport metrics have zero rows, epoch after revision, or offset past maximum | reject as malformed; do not retain its rows or baseline |
| viewport Frame row count differs from `viewport_rows`, exceeds the current model height/80 rows, or exceeds 8 MiB | reject before render; keep the attachment/session bounded |
| viewport request exceeds the 1 MiB control limit | reject before concrete decode or Session dispatch |
| peer lacks `TERMINAL_VIEWPORT` | send no kind 315/316; use unchanged capability-gated history paging or no history feature |
| history-window query has invalid anchor/size/target/margins or exceeds the product viewport | reject as malformed before Session dispatch; no model/cache mutation |
| history-window Frame predates or contradicts its originating query, exceeds 240 rows, or has a mismatched exact range | reject before cache/render; keep the previous complete frame |
| history-window Changed/Gap contains rows/anchor/disposition/coordinates, or nonzero revision predates the request | reject as malformed; content-free outcome only |
| peer lacks `TERMINAL_HISTORY_WINDOW` | send no kind 317/318; fall back to negotiated bit 19 and then bit 17 pager |
| connection status arrives on remote normal ALPN | reject; status is same-UID local IPC only |
| wire major or kind is unsupported | explicit protocol/service error; listener remains healthy |

## 5. Good / Base / Bad Cases

- **Good:** obtain a daemon lease lazily, encode one mutation once, and let
  exactly one transport layer reuse its exact bytes and operation ID for the
  single ambiguous-transport retry.
- **Base:** readiness/status/list use no operation lease and do not alter replay
  state.
- **Good viewport-cache case:** validate a window against its saved complete
  query, present only a full-height cached slice, and retain the prior complete
  presentation while a miss or edge prefetch is outstanding.
- **Bad:** derive an epoch from wall-clock time, accept a client-invented high
  ordinal, wrap a sequence, accept an uncorrelated window shape, or rerun an
  outcome-unknown operation under a fresh lease.

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
- Proto/daemon tests cover additive kinds 312–318, capability bits 17–20,
  alternate-scroll field 7, optional snapshot/delta metrics, all history and
  viewport/window outcomes, page/frame/window bounds, redacted Debug, exact
  request-shape correlation, mixed-version bit20 -> bit19 -> pager fallback,
  and rejection of status on the remote service classifier.
- Core cache tests cover hit-without-request, checked slice math, edge
  prefetch, one complete pending query/latest target, stale response/anchor
  rejection, append translation, resize/identity invalidation, and preservation
  of the last presentable full slice while replacement is unavailable.
- `local_ipc` proves malformed/unsupported requests terminate only their own
  unary connection and do not poison the listener. `terminal_recovery` owns
  the equivalent duplex attachment isolation evidence.

## 7. Wrong vs Correct

### Wrong

```rust
let epoch = wall_clock_nanos();
let id = OperationId::new(epoch, next_sequence());
send(encode_again(id, request)).await?;

send_kind_315_to_every_peer(viewport_request).await?;
send_kind_317_without_saving_its_query(window_request).await?;
```

This has no cross-process authority and can change request bytes between an
ambiguous attempt and its retry.

### Correct

```rust
let lease = client.daemon_issued_lease().await?;
let id = checked_operation_id(lease)?;
let encoded = encode_once(id, request)?;
send_with_one_ambiguous_retry(&encoded, deadline).await?;

if peer_capabilities.contains(Capabilities::TERMINAL_HISTORY_WINDOW) {
    save_query_then_send_window(query).await
} else if peer_capabilities.contains(Capabilities::TERMINAL_VIEWPORT) {
    send_viewport_request(action).await
} else {
    request_legacy_history_page().await
}
```

The daemon validates the incarnation and issued ordinal before effects, while
the client preserves the exact retry identity and payload.

## Forbidden patterns

- Prost, SQLite, Iroh, or OS dependencies in `zterm-core`.
- Raw `u64` revisions in public terminal APIs.
- A second frame parser in the CLI or daemon.
- A host terminal engine dependency or upstream terminal type in core/proto.
- Reusing kind 315/316 or bit 19, emitting them without negotiated
  `TERMINAL_VIEWPORT`, accepting invalid metrics/row counts, or exposing
  viewport row content through Debug.
- Reusing kind 317/318 or bit 20, emitting them without negotiated
  `TERMINAL_HISTORY_WINDOW`, validating a Frame without its originating query,
  treating transport loss as the 0/0 unsupported sentinel, or exposing window
  row content through Debug.
- Re-executing an operation whose result has fallen below the replay low-water
  mark.
- Generating or accepting a client-invented lease ordinal/incarnation, wrapping
  an operation sequence, or automatically retrying an outcome-unknown mutation
  under a fresh lease.
