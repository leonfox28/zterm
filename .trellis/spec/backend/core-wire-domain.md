# Core and Wire Domain Contract

## Scope

Apply this contract when changing shared identifiers, terminal revisions,
capabilities, resource defaults, operation replay, protobuf messages, wire
kinds, or frame encoding. `zterm-core` owns product domain values and
`zterm-proto` owns their wire representation and validation.

## Contracts

- `DeviceId` is 32 bytes; `SessionId` and `AttachmentId` are 16 bytes.
- `Revision` is the only public terminal revision type. It is monotonic and
  checked before mutation.
- `AttachmentPrincipal` distinguishes an authenticated remote endpoint from a
  same-UID local view. A local principal is created only after the platform
  peer-credential gate succeeds.
- Capability values retain unknown bits. Optional future capabilities never
  become prerequisites for ordinary terminal service.
- `DomainErrorKind::code` and `from_code` are the single stable error-category
  bridge used by wire and JSON projections; adapters do not invent aliases.
- `OperationWindow` is fixed to one client epoch and retains exact results in a
  bounded sequence window. A retained duplicate replays its result; an epoch
  mismatch or an evicted sequence returns outcome unknown and is never run.
- M2 owns and tests the replay state machine; M4 first integrates it around
  stateful `SessionService` create/rename/close/takeover commits. M3 lifecycle
  stop signals shutdown only after its response flush and does not claim an
  in-memory replay window.
- `proto/zterm/v1/*.proto` is the wire source of truth. One numeric kind
  registry and one decoder own all message dispatch.
- Frames are `varint length + WireFrame`, capped at 8 MiB before body
  allocation. Control payloads are capped at 1 MiB before concrete-message
  decoding. Unknown protobuf fields are compatible; unknown kind and wire
  major are explicit errors.
- The product ALPN is `zterm/1`. This milestone defines the contract but does
  not bind an Iroh endpoint.

## Forbidden patterns

- Prost, SQLite, Iroh, or OS dependencies in `zterm-core`.
- Raw `u64` revisions in public terminal APIs.
- A second frame parser in the CLI or daemon.
- Re-executing an operation whose result has fallen below the replay low-water
  mark.

## Required evidence

- Core state-machine tests cover ID lengths, principals, unknown capability
  retention, defaults, replay, eviction, errors, and epoch mismatch.
- Proto tests cover round trip, unknown fields, unknown kinds, major mismatch,
  malformed varints, truncated bodies, and both size limits.
- `local_ipc` proves malformed/unsupported requests terminate only their own
  unary connection and do not poison the listener.
