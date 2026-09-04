# Core and Wire Domain Contract

## 1. Scope / Trigger

Apply this contract when changing shared identifiers, terminal revisions and
semantic DTOs, capabilities, operation replay, protobuf messages, wire kinds,
or frame encoding. `zterm-core` owns transport-neutral product values;
`zterm-proto` owns the single wire-major-two representation and structural
validation.

## 2. Signatures

```rust
pub const WIRE_MAJOR: u32 = 2;

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

TerminalSurfaceSnapshot::validate(&self) -> Result<(), TerminalSurfaceError>
TerminalSurfaceDelta::validate(&self) -> Result<(), TerminalSurfaceError>
TerminalSurfaceDelta::apply_to(
    &self,
    current_revision: Revision,
    surface: &mut TerminalSurface,
) -> Result<Revision, TerminalSurfaceError>
TerminalSurfaceHistoryWindowFrame::validate_for(
    &self,
    query: TerminalHistoryWindowQuery,
) -> Result<(), TerminalSurfaceError>

TerminalClipboardWrite::new(text: String)
    -> Result<TerminalClipboardWrite, TerminalClipboardError>
TerminalTextRange::new(anchor: TerminalTextPoint, focus: TerminalTextPoint)
    -> TerminalTextRange
TerminalTextRange::extract(self, rows: &[TerminalSurfaceRow])
    -> Result<TerminalClipboardWrite, TerminalTextSelectionError>

ViewportCache<TerminalSurfaceRow>::set_target(offset: u64)
    -> ViewportCacheUpdate
ViewportCache<TerminalSurfaceRow>::install_window(window)
    -> Result<ViewportCacheInstall, CachedViewportWindow<TerminalSurfaceRow>>
```

The mutation boundary is
`SessionOperationLeaseRequest -> SessionOperationLeaseResponse { lease }`,
followed by a request carrying one exact `OperationId`.

The terminal wire registry is canonical and non-negotiated:

```text
300 TerminalAttachRequest
301 TerminalSemanticSnapshot              content
302 TerminalSemanticDelta                 content
303 TerminalInput
304 TerminalResize
305 TerminalDetach
306 TerminalSnapshotApplied
307 TerminalSyncRequest
308 TerminalSyncRequired
309 TerminalLeaseLost
310 TerminalSessionEnded
311 TerminalTransportStateEvent            same-UID projection
314 TerminalConnectionStatusEvent          same-UID projection
317 TerminalHistoryWindowRequest           control
318 TerminalSemanticHistoryWindowFrame     content
322 TerminalClipboardWrite                 transient host effect
```

The product ALPNs are `zterm/2` and `zterm-pair/2`. Protobuf source/package and
the generated Rust module are exactly `proto/zterm/v2`, `zterm.v2`, and `v2`.

## 3. Contracts

### Shared identities and replay

- `DeviceId` is 32 bytes; `SessionId`, `AttachmentId`, and daemon incarnation
  are 16 bytes. `Revision` is the only public terminal revision type and is
  checked before mutation.
- `OperationWindow` is fixed to one daemon-issued lease and retains exact
  success or typed-error results in a bounded non-zero sequence window. A
  retained duplicate replays; lease mismatch, zero/evicted sequence, payload
  fingerprint mismatch, or exhaustion returns outcome unknown and never runs.
- Lease ordinals are daemon-monotonic per stable principal/auth generation.
  Restart/incarnation mismatch, missing/invented/high/retired ordinal, and a
  retry at or below the completed floor are rejected before effects. Lost empty
  lease responses participate in the same bounded retirement policy.
- Readiness, status, and list allocate no lease. A mutation is encoded once and
  any single ambiguity retry reuses byte-identical request bytes, deadline,
  lease, operation ID, and semantic payload. Outcome unknown is never retried
  under a fresh lease for that logical operation.

### Semantic terminal domain

- `zterm-core::terminal` owns size, screen, cell/style/cursor/modes, side
  events, exact `TerminalSurface`, revision-bound snapshot, full-row semantic
  delta patches, scroll metrics, history-window query/result, and redacted
  Debug implementations.
- `TerminalClipboardWrite` is non-empty UTF-8 text, contains no NUL, is capped
  at 524,288 bytes, and redacts content from Debug. `TerminalHostEffect` carries
  it outside revisioned side events. `TerminalKeyboardFlags` validates exactly
  the five standard Kitty bits and is projected as part of `TerminalModes`.
- A surface has exactly `size.rows` rows and every row has exactly
  `size.columns` cells. Text is bounded, contains no controls, wide head and
  continuation cells form exact adjacent pairs, the cursor is in bounds, and
  scroll metrics are present only for a compatible main-screen surface.
- A delta advances one exact baseline, has sorted unique in-bounds row patches,
  and applies transactionally. Callers install the candidate only after the
  complete delta validates; a mismatch retains the last complete surface and
  requests resynchronization.
- History-window requests contain an immutable epoch/revision/extent/viewport
  anchor, absolute target, and bounded margins. `response_shape` is the single
  authority for disposition, translated target, signed first row, and exact row
  count. A Frame contains semantic rows only; Changed/Gap are content-free and
  report `epoch <= revision` with `revision >= query.anchor.revision`.
- `ViewportCache<TerminalSurfaceRow>` is renderer-neutral and contains no ANSI,
  async runtime, mouse pixels, platform type, or terminal parser. It retains one
  complete bounded window, desired/presented offsets, latest anchor, and one
  complete outstanding query; later gestures coalesce to the latest target. Its
  immutable visible/presented slice identities let an attachment prove whether
  selected history coordinates still address the same semantic rows across a
  compatible monotonic append.
- `TerminalTextPoint`/`TerminalTextRange` are renderer-neutral inclusive cell
  coordinates. Normalization is direction-independent; extraction expands wide
  glyph endpoints, emits each wide head once, preserves combining contents,
  maps selected blank cells to spaces, joins wrapped rows, separates unwrapped
  rows with newline, and enforces the clipboard cap incrementally and atomically.
- No public `TerminalState`, presentation-family wrapper, encoding preference,
  legacy ANSI snapshot/delta/history DTO, or stateful server viewport action
  exists.

### Wire-major-two representation

- Semantic snapshot, delta, and history-window frame are the only terminal
  content representations. Terminal attach has no presentation preference.
  Capabilities retains unknown bits, but no bit 17/19/20/21 presentation
  negotiation or fallback exists; `TERMINAL_SERVICE` is the only terminal
  service capability.
- Kinds 312/313, 315/316, and 319/320/321 are retired and must not appear in the
  v2 registry. Kind 318 means only semantic history-window response; kind 322
  means only the structured transient clipboard write.
- `proto/zterm/v2/*.proto` is the only compiled wire source. There is no v1
  generated module, dual decoder, downgrade, compatibility adapter, or
  mixed-version terminal branch. Independently persisted formats such as
  `PairTicketV1` and `RelayRouteCacheV1` keep their own version names and do not
  imply wire-v1 support.
- Frames are `varint length + WireFrame`, capped at 8 MiB before body
  allocation. Concrete control payloads are capped at 1 MiB before decoding;
  kinds 301, 302, and 318 use the content-frame limit. Kind 322 is control,
  always uses request ID zero, and carries an exact attachment ID plus decoded
  structured text; its domain cap remains stricter than the control cap.
  Unknown protobuf fields remain compatible, while unknown kind or wire major
  is an explicit connection-local error.
- Model/driver/Session/local IPC/remote bridge pass semantic values only. A
  bridge may structurally decode cells to validate shape, content bounds,
  revision, correlation, and request identity, rewrite its private attachment
  ID, then re-encode. It must not interpret application content, convert
  representation, construct ANSI, or perform presentation.
- Raw child OSC 52 never crosses the terminal ingress boundary. A clipboard
  effect is controller-at-publication-time, latest-only, non-broadcast,
  non-replayable, and absent from snapshots, deltas, history, checkpoints,
  operation leases, persistence, and formatted diagnostics. Attachment bridges
  may rewrite only their private attachment ID after revalidation.
- `TerminalConnectionStatusEvent` is same-UID only and contains attachment ID,
  unknown/direct/relay, and optional bounded integer RTT. It is invalid on the
  remote normal ALPN and contains no address, relay URL, DeviceId, or ticket.

## 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| identifier/incarnation has wrong byte length | reject during domain conversion before dispatch |
| replay capacity is zero | `OperationWindowError::InvalidCapacity` |
| lease/sequence/fingerprint is missing, mismatched, retired, evicted, or exhausted | outcome unknown/exhaustion; do not execute |
| frame prefix is malformed/non-canonical, body truncated, kind unknown, or wire major not 2 | connection-local protocol error; listener remains healthy |
| frame exceeds 8 MiB or control payload exceeds 1 MiB | reject before concrete-message allocation/dispatch |
| semantic surface has invalid size/row count/cell text/wide pair/cursor/metrics | reject transactionally; retain previous complete surface |
| semantic delta does not advance exact baseline or has duplicate/out-of-range patches | reject; request full sync and do not partially apply |
| history query has invalid anchor/size/target/margins | reject before Session/model/cache mutation |
| history Frame contradicts its query, exceeds 240 rows, or has invalid semantic rows | reject before cache/presentation; retain prior complete window |
| Changed/Gap contains rows/shape fields, has epoch after revision, or revision older than query anchor | reject as malformed; content-free outcome only |
| clipboard text is empty, contains NUL, or exceeds 524,288 UTF-8 bytes | reject before effect routing/wire output; expose no content in Debug/error |
| terminal keyboard flags contain unknown bits | reject during protocol conversion; never silently mask or invent child state |
| kind 322 has nonzero request ID, missing/wrong attachment ID, or invalid clipboard text | reject at the attachment boundary; never reinterpret it as raw OSC or ordinary replayable control |
| selection endpoint is out of bounds or splits a valid wide glyph | reject invalid coordinates / expand valid head-continuation endpoints to the whole glyph before extraction |
| connection status arrives on remote normal ALPN | reject; status is same-UID local IPC only |
| old v1 ALPN or wire major is used | explicit incompatibility; never enter terminal attachment or downgrade |

## 5. Good / Base / Bad Cases

- **Good:** encode a mutation once and let exactly one transport owner reuse its
  exact bytes and operation ID for the one allowed ambiguity retry.
- **Base:** latest v2 client and daemon exchange semantic snapshot/delta/history
  with no presentation negotiation.
- **Good:** validate a history response against its saved complete query, then
  install only one complete semantic row window.
- **Bad:** accept `(epoch, revision) = (0, 0)` as a generic unsupported sentinel,
  infer a response without its query, send a retired kind, or recreate ANSI in
  the daemon/bridge.

## 6. Tests Required

- Core tests cover ID lengths, principals, unknown capability retention,
  replay/eviction/exhaustion, semantic surface/delta/history validation,
  clipboard empty/NUL/exact-cap/over-cap and redacted Debug, all keyboard flag
  combinations/unknown bits, range direction/wide/combining/blank/wrap/cap
  extraction, and renderer-neutral cache/slice-identity transitions.
- Proto tests cover v2 round trip, unknown fields/kinds, major mismatch,
  non-canonical/malformed varints, truncated bodies, both size limits, exact
  kind registry including 322, semantic Unicode/wide/style rows, request-bound
  history, malformed Changed/Gap epoch/revision identity, clipboard
  ID/text/request-zero validation and redaction, and unknown keyboard bits.
- Daemon/local/remote tests trace kinds 301/302/317/318/322 through initial full,
  merged delta, gap/resync, reconnect, takeover, final drain, correlation/ID
  rewrite, stream-loss Gap, controller-at-event-time clipboard targeting,
  latest-only wakeup, observer exclusion, and no replay. No
  negotiation/fallback matrix remains.
- `tests/source-policy.sh` rejects v1 protobuf, retired terminal kinds, legacy
  presentation types, `TerminalState`, a second CLI parser, application-name
  detection, and Zterm-owned unsafe Rust.

## 7. Wrong vs Correct

### Wrong

```rust
if peer_capabilities.contains(LEGACY_VIEWPORT) {
    send_kind_315(action).await?;
} else {
    request_ansi_history_page().await?;
}

bridge.write(encode_ansi(decoded_cells));
```

### Correct

```rust
let query = cache.set_target(target).request;
if let Some(query) = query {
    save_query_then_send_kind_317(query).await?;
}

let frame = decode_validate_history_kind_318(saved_query, bytes)?;
let rewritten = frame.with_attachment_id(local_view_id);
forward_semantic(rewritten).await?;
```

The boundary may decode structurally, but only the platform presenter interprets
the semantic surface for physical output.

## Forbidden patterns

- Prost, SQLite, Iroh, OS, or terminal-engine dependencies in `zterm-core`.
- Raw `u64` revisions in public terminal APIs.
- A second frame parser in the CLI/daemon or a host-engine type in core/proto.
- Wire-v1 source/generated modules, presentation capability bits, family or
  encoding negotiation, retired kinds, downgrade, or speculative fallback.
- ANSI-bearing terminal DTOs or ANSI construction outside the sole desktop
  presenter.
- Raw OSC clipboard bytes on the wire, clipboard payload in a watch/replay
  queue/revision, nonzero request IDs for kind 322, or payload-bearing
  clipboard Debug/error output.
- Desktop gestures, mouse pixels, Kitty parsing, ANSI, or clipboard backends in
  the core terminal range/extraction helpers.
- Validating a history response without its complete originating query or
  treating stream loss as an uncorrelated/sentinel response.
- Re-executing an operation below the replay low-water mark or under a new lease
  after an outcome-unknown result.
