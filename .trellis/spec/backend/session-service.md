# Persistent Session Service Contract

## 1. Scope / Trigger

Apply this contract to `zterm-daemon::session`, live-session resource
admission, controller attachments, and daemon shutdown. The service is the
single transport-independent owner used by same-UID local IPC now and by a
future authenticated remote adapter.

## 2. Signatures

```rust
SessionService::list(&self) -> Result<Vec<SessionSummary>, DaemonError>
SessionService::create(&self, principal, operation_id, name, cwd, viewport)
    -> Result<SessionSummary, DaemonError>
SessionService::rename(&self, principal, operation_id, session_id, new_name)
    -> Result<SessionSummary, DaemonError>
SessionService::close(&self, principal, operation_id, session_id)
    -> Result<SessionSummary, DaemonError>
SessionService::prepare_attach(&self, selector, create_main, takeover, viewport)
    -> Result<PreparedAttachment, DaemonError>
SessionService::takeover(&self, principal, operation_id, attachment)
    -> Result<SessionSummary, DaemonError>
SessionService::shutdown(&self) -> Result<Vec<SessionSummary>, DaemonError>

SessionAttachment::snapshot_applied(revision) -> Result<(), DaemonError>
SessionAttachment::next_update() -> Result<Option<AttachmentUpdate>, DaemonError>
SessionAttachment::write_input(bytes) -> Result<(), DaemonError>
SessionAttachment::resize(size) -> Result<Revision, DaemonError>
```

Adapters may call the deadline-bearing `*_until(..., Instant)` variants, but
they must not duplicate registry, replay, resource, or controller logic.

## 3. Contracts

- A `SessionService` owns one in-memory ID/name registry per daemon. A session
  owns one login-shell PTY, one `TerminalDriver`, attachment checkpoints, one
  controller lease, and one fixed-cell resource reservation.
- M4 keeps the transport-independent service API synchronous, but each live
  session is owned by one dedicated OS-thread `SessionActor` with a fixed
  16-command synchronous mailbox. PTY write/flush, resize, snapshots, model
  synchronization, attachment changes, and finalization run only on that
  worker. A blocked session A cannot hold a registry lock, run on the Tokio
  socket thread, or prevent session B's worker from progressing.
- Every actor command carries one absolute `Instant` deadline and a shared
  queued/started/expired gate. Socket code enters synchronous service paths only
  through `spawn_blocking`, and actor admission uses non-blocking `try_send`
  retries rather than awaiting a full mailbox on the current-thread runtime. If
  the caller wins queued-to-expired, the worker must not begin its side effect;
  if the worker already won queued-to-started, a disconnect or timeout drops
  only the waiter and the accepted mutation continues to an exact replayable
  result.
- Session state is daemon-lifetime only. Disconnect and detach preserve the
  PTY; root-shell exit, explicit close, and daemon stop end it. No transcript,
  PTY, attachment, lease, or operation result is written to SQLite or disk.
- The default attach path reserves `main` once under the registry's atomic name
  slot and uses a per-name creation cell for singleflight. Normal create and
  rename cannot claim `main`; closing it permits the next default attach to
  create a new ID.
- `SessionName` is the only name validator: UTF-8, 1–64 bytes, case-sensitive,
  no surrounding whitespace, and no Unicode control characters.
- A create first owns one unforgeable in-process token. Under the single lock
  order `registry state -> resources`, candidate `SessionId` collision checks,
  ID reservation, resource insertion, and association with the `Starting`
  name slot are atomic; insertion cannot overwrite another projection. The
  started actor transfers to `CreationOwner` before ordinary provisional
  registration can fail. Name, resource, provisional, and live entries carry
  the same token and cleanup uses compare-and-remove, so an unrelated owner is
  never released. Rename checks the same name-slot map, so create-vs-rename can
  never publish duplicate names.
- Cancellation prevents publication but does not remove a `Starting` slot
  which owns a resource/actor. Publication loss explicitly interrupts, reaps,
  drains, and joins the new driver before releasing its token and name. If
  bounded cleanup or registration fails, a provisional/cleanup-only actor stays
  registry-visible and the original name remains unavailable until the actor
  finalizer proves child/thread release. Shutdown enumerates all such owners.
  No registry lock is held while spawning, waiting, finalizing, or writing a
  socket.
- Admission permits at most eight sessions, 2,000 history rows each, a
  240-column by 80-row viewport, and 128 MiB summed fixed-cell projection.
  Missing viewport uses 120 columns by 40 rows. Resize reserves the projection
  delta before native/model resize and rolls it back on failure.
- One controller is allowed per session. A normal second attach is occupied.
  Explicit takeover first creates and synchronizes a pending attachment, then
  atomically increments generation, invalidates the old lease, and activates
  the replacement. If its response is lost, a newly synchronized attachment
  may continue the same opaque operation: it becomes controller when the
  controller is absent or still tagged by that operation, but never replaces a
  controller installed by a later/different operation. At most one pending
  takeover exists per session, so 1.0 retains at most the active controller
  checkpoint and one replacement checkpoint. Attachments never receive PTY
  close authority.
- Every full snapshot must be acknowledged at its exact revision before input
  or resize. A mismatch discards the checkpoint and returns a latest snapshot;
  input is not queued while synchronization is pending.
- Revision and lifecycle notification use Tokio `watch` watermarks. There is
  no per-revision backlog. A slow view receives one merged delta or a full
  replacement snapshot. An adapter must inspect the current lifecycle value
  immediately after subscribing; a transition which happened before
  `subscribe()` is already considered seen by `changed()`.
- Each attachment checkpoint reconstructs only the current main and alternate
  visible grids in a zero-scrollback parser. It does not clone host history;
  checkpoint capacity is fixed at `rows * columns * 2` cells independent of the
  configured 2,000 history rows. Main/alternate transitions, styles, Unicode,
  and resize-to-resync remain semantically equivalent to the latest snapshot.
- `SessionEnded` is terminal for an attachment. A stale snapshot acknowledgement,
  sync request, or prepared takeover must fail without replacing that lifecycle
  watermark or making the attachment live again. When revision and lifecycle
  notifications race, the terminal lifecycle event wins. The owner finalizes
  PTY/model drain first, and the socket writer emits one last merged update
  before `SessionEnded`, so root-shell tail output is not discarded.
- Create, rename, close, and takeover register through a short global replay
  lookup and then coordinate on one fingerprinted per-operation cell. The
  winner executes outside the replay registry lock; the same ID and semantic
  payload joins/replays its complete success or typed error, a different
  payload is outcome unknown, and unrelated keys run concurrently. A panic or
  dropped executor terminally completes the cell with outcome unknown, so
  waiters cannot hang and the lease can retire. Each lease retains 128 result
  cells and the registry retains 64 active leases.
- The daemon issues each lease with its random daemon incarnation and a strictly
  increasing ordinal per stable principal/auth generation. Lost lease responses
  may leave empty leases; fully completed prefixes, including empty leases,
  retire into an exact per-principal floor. In-flight results are never retired.
  Restart/incarnation mismatch, invented/high/missing ordinal, and a retry at or
  below the floor return `operation_outcome_unknown` before any side effect.
  Ordinal and operation-sequence exhaustion are explicit and never wrap. The
  stable local identity ignores socket-view ID and relies on the same-UID peer
  gate plus daemon device identity (authorization generation zero).
- Readiness, status, and list do not allocate a lease or mutate replay state.
  Ordinary local mutations lazily allocate and cache one. A transport-ambiguous
  retry reuses the identical encoded request and operation ID at most once. A
  typed outcome-unknown response poisons that lease for the client, is never
  auto-retried under another lease, and only a later independent user operation
  may allocate a replacement.
- Actor worker, close thread, creation owner, spawned PTY, and driver ownership
  have unwind finalizers. Cleanup locks recover poison, then compare the exact
  actor/token; normal operations may still report synchronization failure. A
  panic completes replay/creation waiters truthfully, leaves unfinished
  ownership visible, and makes cleanup retriable; it cannot strand a matching
  name reservation, resource projection, pending interrupt, worker end state,
  child, or registry entry. `Drop` never joins inline: it first interrupts and
  aborts, exclusively takes any handle, and hands it to a self-join-safe
  background reaper. Actor registry ownership is released only after the
  driver's child/thread completion signal, not merely after reaper admission.
- Root exit and explicit close may race. Ending is idempotent and registry
  removal is compare-and-remove against the same actor, so resource release
  occurs once and cannot remove a later session reusing the name.
- Explicit close uses child control separated from the potentially blocked PTY
  writer. Daemon shutdown requests interruption for every live/provisional
  owner before processing any summary error, then joins completed workers under
  one absolute deadline. Only an ordinary ended `session_not_found` summary
  race is omitted; every other typed summary/finalization error is surfaced
  after all owners received close. A stuck child/driver yields
  `deadline_exceeded`; ownership stays visible, admission/listener/socket remain
  available for diagnosis and retry, and `stopping=true` is never sent until
  every child, driver thread, actor, and reservation is released.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| invalid/reserved/conflicting name | typed error, no PTY or index mutation |
| invalid or inaccessible cwd | `invalid_working_directory`, no publication |
| ninth session, invalid viewport, aggregate projection overflow | `resource_exhausted` |
| normal second controller | `session_occupied` |
| overlapping first attach after one request owns/pends the controller | `session_occupied`; the registry still contains exactly one `main` |
| input/resize before exact snapshot acknowledgement | `not_synchronized` |
| stale/replaced attachment | `lease_lost`, no PTY write |
| missing session selector | `session_not_found` |
| retained operation retry | exact prior success or typed error |
| invalid incarnation/ordinal, retired lease, or evicted sequence | `operation_outcome_unknown`, no side effect |
| same operation ID with a different semantic payload | `operation_outcome_unknown`, no replay or side effect |
| command expires while queued | `deadline_exceeded`, no PTY/model/lease side effect |
| shutdown deadline with owned session remaining | `deadline_exceeded`, daemon remains available |

## 5. Good / Base / Bad Cases

- **Good:** create owns a name/ID/resource token atomically, publishes only after
  actor startup, and retains that ownership visibly until child and threads are
  proven released.
- **Base:** detaching the only controller leaves the PTY draining and the
  daemon-lifetime session available for a later snapshot/resync.
- **Good retry case:** a response-lost takeover reconnects with the same opaque
  operation and may replace only the controller established by that operation.
- **Bad:** hold a registry or replay lock across spawn/PTTY I/O, run a blocking
  session effect on the current-thread Tokio runtime, remove a `Starting` name
  before cleanup succeeds, or join worker threads from `Drop`.

## 6. Tests Required

- `session_lifecycle` covers concurrent `main`, detach/reconnect, named
  lifecycle, invalid cwd, replay, natural root exit, close, and recreation. Its
  concurrent-first-attach case requires one `main`, at least one successful
  attachment, identical SessionIds for all successes, and permits the remaining
  overlapping controller requests to return `session_occupied`.
- `controller_lease` covers occupied, prepared takeover, lease loss, no stale
  write, same-operation continuation after controller detach, and rejection of
  a stale same-operation continuation without clobbering a later controller.
- `session_limits` covers the eighth/ninth session, maximum viewport, aggregate
  projection, and failed-resize rollback.
- `local_session_ipc` and `terminal_recovery` prove the same service through a
  real peer-authorized Unix socket, including daemon stop and connection-local
  protocol failure.
- `session_concurrency` and session unit tests prove unrelated replay overlap,
  same-key join, create/rename reservation, publication-loss cleanup, blocked
  actor deadline isolation, issued-lease retirement/restart/exhaustion,
  panic-safe replay waiters, unwind cleanup, provisional publication ownership,
  atomic ID/resource collision handling, cleanup-timeout name retention,
  poison-aware compare removal, lock-order concurrency, and truthful concurrent
  shutdown/error collection. `local_session_ipc`
  additionally proves takeover continuation after real response loss, a
  blocked PTY does not stall status/session B, recoverable listener accept
  failures preserve live ownership, fatal listener exit rebinds the exact owned
  socket until stop is retryable, and failed bounded stop leaves the listener
  available. `local_ipc` proves byte-identical single-retry execution and that
  typed outcome unknown rotates only for a later operation.
- Concurrent shutdown evidence observes that every actor has received its end
  request before cleanup waiting, then separately requires all child PIDs and
  ownership to be released under the final absolute deadline. It must not use
  a fixed process-reap interval shorter than the platform PTY library's signal
  grace period as a proxy for concurrent initiation.

## 7. Wrong vs Correct

### Wrong

```rust
let mut registry = registry.lock()?;
let result = actor.write_all(input); // may block while every session waits
registry.remove(&session_id);
result
```

### Correct

```rust
let actor = registry.lookup(session_id)?; // short lock only
let command = Command::input(input, absolute_deadline);
actor.try_submit(command)?;               // bounded per-session mailbox
```

Creation and cleanup additionally compare one ownership token across the name,
provisional/live actor, and resource projection before removing anything.

## Forbidden patterns

- A registry in an adapter, CLI, GUI, or future Iroh transport.
- Closing a PTY because a stream, socket, tab, or attachment disappeared.
- Per-program branches for tmux, Herdr, Codex, OpenCode, or other hosted tools.
- Per-revision queues, transcript persistence, arbitrary launch commands, or
  Agent-specific state branches in the 1.0 terminal service.
- Holding a global replay mutex while running a side effect, retiring an
  in-flight operation, accepting a client-invented lease, or replaying one
  operation ID for a different payload.
- Awaiting a full session mailbox on the current-thread Tokio runtime, or
  reporting/flushing a successful stop while owned session resources remain.
