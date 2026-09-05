# Persistent Session Service Contract

## 1. Scope / Trigger

Apply this contract to `zterm-daemon::session`, live-session count/dimension
admission, controller attachments, and daemon shutdown. The service is the
single transport-independent owner used by same-UID local IPC and the
authenticated remote adapter.

## 2. Signatures

```rust
SessionService::list(&self) -> Result<Vec<SessionSummary>, DaemonError>
SessionService::create(&self, principal, operation_id, name, cwd, viewport)
    -> Result<SessionSummary, DaemonError>
SessionService::rename(&self, principal, operation_id, session_id, new_name)
    -> Result<SessionSummary, DaemonError>
SessionService::close(&self, principal, operation_id, session_id)
    -> Result<SessionSummary, DaemonError>
SessionService::prepare_attach(&self, principal, selector, create_main, takeover, viewport)
    -> Result<PreparedAttachment, DaemonError>
SessionService::takeover(&self, principal, operation_id, attachment)
    -> Result<SessionSummary, DaemonError>
SessionService::detach_remote_principal(&self, device_id: DeviceId)
    -> Result<PrincipalDetachImpact, DaemonError>
SessionService::shutdown(&self) -> Result<Vec<SessionSummary>, DaemonError>

SessionAttachment::snapshot_applied(revision) -> Result<(), DaemonError>
SessionAttachment::next_update() -> Result<Option<AttachmentUpdate>, DaemonError>
SessionAttachment::write_input(bytes) -> Result<(), DaemonError>
SessionAttachment::resize(size) -> Result<Revision, DaemonError>
SessionAttachment::history_window(query: TerminalHistoryWindowQuery)
    -> Result<TerminalSurfaceHistoryWindowResult, DaemonError>
SessionAttachment::effect_watch() -> watch::Receiver<()>
SessionAttachment::take_host_effect() -> Result<Option<TerminalHostEffect>, DaemonError>
```

The two transient host-effect delivery methods and their attachment fields are
`cfg(unix)`. Windows keeps the shared Session/model boundary constructible but
stores and subscribes to no unsupported host effect.

Adapters may call the deadline-bearing `*_until(..., Instant)` variants, but
they must not duplicate registry, replay, session-reservation, or controller logic.

## 3. Contracts

- A `SessionService` owns one in-memory ID/name registry per daemon. A session
  owns one login-shell PTY, one `TerminalDriver`, attachment checkpoints, one
  controller lease, one `zterm-terminal` authoritative model, and one
  session-count reservation.
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
- Every prepared/live attachment and controller lease carries an explicit
  `AttachmentPrincipal`; adapters never infer it from a session name or target.
  `detach_remote_principal(_until)` fans out under one deadline and removes only
  attachments, prepared takeover state, and the controller lease matching one
  remote `DeviceId`. It does not signal the child, interrupt the driver, close
  the Session, remove its model/registry entry, or affect local/other-remote
  principals. The device-revoke coordinator calls it only after durable revoke,
  registry publication, and broker close.
- The default attach path reserves `main` once under the registry's atomic name
  slot and uses a per-name creation cell for singleflight. Normal create and
  rename cannot claim `main`; closing it permits the next default attach to
  create a new ID.
- `SessionName` is the only name validator: UTF-8, 1–64 bytes, case-sensitive,
  no surrounding whitespace, and no Unicode control characters.
- A create first owns one unforgeable in-process token. Under the single lock
  order `registry state -> session reservations`, candidate `SessionId`
  collision checks, ID/count reservation, and association with the `Starting`
  name slot are atomic; insertion cannot overwrite another owner. The
  started actor transfers to `CreationOwner` before ordinary provisional
  registration can fail. Name, reservation, provisional, and live entries carry
  the same token and cleanup uses compare-and-remove, so an unrelated owner is
  never released. Rename checks the same name-slot map, so create-vs-rename can
  never publish duplicate names.
- Cancellation prevents publication but does not remove a `Starting` slot
  which owns a reservation/actor. Publication loss explicitly interrupts, reaps,
  drains, and joins the new driver before releasing its token and name. If
  bounded cleanup or registration fails, a provisional/cleanup-only actor stays
  registry-visible and the original name remains unavailable until the actor
  finalizer proves child/thread release. Shutdown enumerates all such owners.
  No registry lock is held while spawning, waiting, finalizing, or writing a
  socket.
- Admission permits at most eight sessions, fixes model history at 2,000 rows,
  and accepts at most a 240-column by 80-row viewport. Missing viewport uses
  120 columns by 40 rows. Create reserves only the session identity/count;
  resize validates dimensions and revision before native/model mutation. No
  estimated terminal-memory total participates in create or resize.
- One controller is allowed per session. A normal second attach is occupied.
  Principal kind has no priority: same-UID local cannot implicitly replace a
  remote controller, and remote cannot implicitly replace local. Both must use
  the same explicit takeover protocol against the same SessionId/PTY.
  Explicit takeover first creates and synchronizes a pending attachment, then
  atomically increments generation, invalidates the old lease, and activates
  the replacement. If its response is lost, a newly synchronized attachment
  may continue the same opaque operation: it becomes controller when the
  controller is absent or still tagged by that operation, but never replaces a
  controller installed by a later/different operation. At most one pending
  takeover exists per session, so 1.0 retains at most the active controller
  checkpoint and one replacement checkpoint. Attachments never receive PTY
  close authority.
- The Session actor is also the sole authority for transient host-effect
  eligibility. One centralized reconciliation derives a target only for the
  current generation after first activation, or for that exact previously
  active controller during an in-epoch replacement snapshot. A fresh or
  prepared takeover is ineligible. First activation is attachment-lifetime
  state: resume or takeover never inherits eligibility merely because the
  terminal or an older attachment was previously active. Takeover, detach,
  principal removal, resume, lease loss, and Session end reconcile the broker
  under its single target lock; changing target clears pending content. The
  effect subscription exists before an eligible target is installed, and target
  installation precedes publishing the externally observable `Active`
  lifecycle value. Effects are event-time targeted, best-effort, latest-only,
  and absent from snapshot, delta, history, checkpoint, resume, replay,
  persistence, and final drain.
- A fresh attach or pending takeover must acknowledge its first full semantic
  snapshot at the exact revision before input or contiguous history-window
  operations.
  ACK while not Awaiting remains `not_synchronized`. An ordinary streamed
  delta is never an ACK barrier: a queued stale delta ACK can first cause a
  replacement snapshot while Awaiting, then make the duplicate exact ACK fail
  after activation. Fix the client update-origin contract rather than relaxing
  this target authority.
  A mismatch discards the checkpoint and returns a latest snapshot; input is
  not queued by the Session. Resize is replaceable controller state and retains
  its existing broader allowance while an Active-target snapshot is in flight.
  Once the attachment has been Active, the same current controller/generation
  may also issue input and history-window operations across a later
  replacement-snapshot `Awaiting -> Active` window. This narrow
  `ever_active` fence handles the duplex ordering race without admitting a
  fresh attachment, pending takeover, stale generation, or different
  controller. Contiguous live deltas may arrive before an activation barrier;
  adapters advance the successfully applied revision through that chain, while a gap
  requests a fresh authoritative sync and never silently activates it.
- A contiguous history-window request is controller-only and passes one fully
  validated anchor/target/margin query through the same initial-versus-
  replacement sync fence. It is stateless at
  the Session boundary: it does not read or update a scroll position,
  checkpoint, resume state, controller lease, PTY, or revision delivery. Query
  correlation/coalescing belongs to the local/remote adapter and client cache.
- `next_update` and the final-drain path call the driver's `sync_changed`, not
  mandatory `sync_latest`. An attachment checkpoint equal to the current model
  revision is a no-op and emits no frame. Initial attach, explicit sync, and
  reconnect retain the mandatory full-sync API.
- Prepared-takeover readiness is independent of current replacement-snapshot
  ordering. `prepared_snapshot_applied` proves that this attachment applied a
  prepared snapshot in the current attachment lifetime. A takeover response
  may arrive before or after the current snapshot acknowledgement, but the
  attachment becomes Active only when takeover no longer needs a response and
  the current snapshot is acknowledged. A reconnect/new attachment resets this
  readiness; it never crosses an attachment or stream epoch.
- Revision and lifecycle notification use Tokio `watch` watermarks. There is
  no per-revision backlog. A slow view receives one merged delta or a full
  replacement snapshot. An adapter must inspect the current lifecycle value
  immediately after subscribing; a transition which happened before
  `subscribe()` is already considered seen by `changed()`.
- Each attachment checkpoint stores one Zterm-owned projection of the latest
  active viewport. It holds neither Alacritty engine state, inactive-screen
  state, nor host history; checkpoint capacity is fixed at `rows * columns`
  cells independent of the configured 2,000 history rows. Main/alternate
  transitions resynchronize, while styles, Unicode, and resize-to-resync remain
  semantically equivalent to the latest snapshot.
- An authenticated remote controller may move exactly one visible checkpoint
  into the Session's bounded resume cell only when its authenticated reader
  reaches clean EOF with no partial frame. The key binds the accepted principal
  generation, SessionId, and daemon-issued resume-view ID. Transport I/O loss,
  explicit detach, protocol failure, terminal lifecycle, generation mismatch,
  and replacement discard instead of saving it. A reconnect with the exact
  retained revision receives one merged delta; every mismatch or missing cell
  receives the authoritative full snapshot. The resume cell retains no host
  history, PTY bytes, per-revision queue, or disk state.
- `SessionEnded` is terminal for an attachment. A stale snapshot acknowledgement,
  sync request, or prepared takeover must fail without replacing that lifecycle
  watermark or making the attachment live again. When revision and lifecycle
  notifications race, the terminal lifecycle event wins. The owner finalizes
  PTY/model drain first, and the socket writer emits one last merged update
  before `SessionEnded`, so root-shell tail output is not discarded.
- An explicit detach racing a writable revision-only delta remains an explicit
  detach, not transport loss. If the writer first observes the closed peer,
  the attachment server gives its already-running reader at most the existing
  operation timeout to classify the queued detach or clean EOF; it adds no
  unbounded wait and saves no remote resume checkpoint for explicit detach.
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
  name/session-count reservation, pending interrupt, worker end state,
  child, or registry entry. `Drop` never joins inline: it first interrupts and
  aborts, exclusively takes any handle, and hands it to a self-join-safe
  background reaper. Actor registry ownership is released only after the
  driver's child/thread completion signal, not merely after reaper admission.
- Root exit and explicit close may race. Ending is idempotent and registry
  removal is compare-and-remove against the same actor, so reservation release
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
| ninth session or invalid viewport | `resource_exhausted`; no PTY/model mutation |
| normal second controller | `session_occupied` |
| overlapping first attach after one request owns/pends the controller | `session_occupied`; the registry still contains exactly one `main` |
| fresh/takeover input or history-window request before exact first snapshot acknowledgement | `not_synchronized`; no PTY/model/checkpoint mutation |
| previously-active current controller issues input/history-window during a later replacement snapshot | admit exactly once against the same generation; do not activate or acknowledge the snapshot implicitly |
| history-window request is non-controller, alternate-screen, stale, invalid, or out of bounds | typed lease/sync/changed/gap/malformed result; no PTY or checkpoint mutation |
| steady-state checkpoint equals current model revision | `next_update == None`; publish no `SyncRequired`/snapshot loop |
| takeover response arrives before current snapshot acknowledgement | retain pending activation; activate only after that acknowledgement |
| stale/replaced attachment | `lease_lost`, no PTY write |
| host effect occurs before first activation or with no controller | drop; never replay after acknowledgement/reconnect |
| host effect races takeover/detach | the broker-lock order selects exactly the old or new eligible controller; never both |
| missing session selector | `session_not_found` |
| retained operation retry | exact prior success or typed error |
| invalid incarnation/ordinal, retired lease, or evicted sequence | `operation_outcome_unknown`, no side effect |
| same operation ID with a different semantic payload | `operation_outcome_unknown`, no replay or side effect |
| command expires while queued | `deadline_exceeded`, no PTY/model/lease side effect |
| shutdown deadline with owned session remaining | `deadline_exceeded`, daemon remains available |

## 5. Good / Base / Bad Cases

- **Good:** create owns a name/ID/session-count token atomically, publishes only after
  actor startup, and retains that ownership visibly until child and threads are
  proven released.
- **Base:** detaching the only controller leaves the PTY draining and the
  daemon-lifetime session available for a later snapshot/resync.
- **Good retry case:** a response-lost takeover reconnects with the same opaque
  operation and may replace only the controller established by that operation.
- **Good visual-sync case:** allow only the exact already-active current
  controller to continue input/history requests during an in-epoch replacement
  snapshot, while activation still waits for the authoritative acknowledgement.
- **Good window case:** authorize one immutable query, return the model result,
  and retain no scroll target so two clients cannot share or move one another's
  cache position.
- **Bad:** hold a registry or replay lock across spawn/PTTY I/O, run a blocking
  session effect on the current-thread Tokio runtime, remove a `Starting` name
  before cleanup succeeds, persist a scroll baseline in the model/resume cell,
  or join worker threads from `Drop`.

## 6. Tests Required

- `session_lifecycle` covers concurrent `main`, detach/reconnect, named
  lifecycle, invalid cwd, replay, natural root exit, close, and recreation. Its
  concurrent-first-attach case requires one `main`, at least one successful
  attachment, identical SessionIds for all successes, and permits the remaining
  overlapping controller requests to return `session_occupied`.
- `controller_lease` covers occupied, prepared takeover, lease loss, no stale
  write, same-operation continuation after controller detach, and rejection of
  a stale same-operation continuation without clobbering a later controller.
  Its cross-principal PTY case performs remote -> local -> remote takeover on
  one SessionId, preserves process/cwd/screen continuity, proves ordinary
  attach never steals in either direction, and keeps another principal's
  independent Session progressing.
- Session/unit wire tests cover bounded local and authenticated-remote semantic
  history-window routing, exact attachment/controller authorization, malformed
  request shape, and read-only behavior alongside interleaved live revisions.
- Session/unit wire tests also cover exact attachment checkpoints,
  automatic-resync behavior, explicit-sync reset,
  first-attach/takeover rejection, and the exact already-active controller
  allowance across an `Awaiting` replacement window. Input in that allowed
  window must reach the PTY exactly once.
- Session/unit wire tests cover contiguous-window controller and sync
  authorization, request bounds, exact/rebased/gap/alternate results, read-only
  behavior across live revisions, and proof that checkpoint and another
  attachment are unchanged.
- Takeover tests cover response-before-ack, ack-before-response, reconnect
  readiness reset, and same-operation continuation without allowing a later
  controller to be replaced. Driver/session tests cover equal-revision no-op so
  snapshot acknowledgement cannot start an infinite resync loop.
- Host-effect tests cover first-sync exclusion, already-active replacement
  sync, takeover target replacement, detach/principal removal, no-controller
  drop, latest-wins, observer exclusion, and no replay. Lifecycle assertions
  must observe the broker target commit before `Active`/lease replacement is
  externally visible.
- `principal_detach`, `local_device_ipc`, and the `session_wire` revoke matrix
  cover matching-only detach across Sessions, stale-effect rejection,
  idempotence, preservation of local/other-remote attachments, durable restart
  state, and survival of the same Session/PTY.
- `session_limits` covers the eighth/ninth session, maximum viewport, resize
  validation without mutation, and the absence of terminal-memory admission.
  Its rejected-resize assertion compares the stable Session identity and
  accepted viewport, not the whole `SessionSummary`: PTY output may
  independently advance the authoritative terminal revision between two
  `list` calls. Do not add a sleep or quiescence barrier to freeze unrelated
  output merely to make that assertion byte-for-byte equal.
- `local_session_ipc` and `terminal_recovery` prove the same service through a
  real peer-authorized Unix socket, including daemon stop and connection-local
  protocol failure.
- `session_concurrency` and session unit tests prove unrelated replay overlap,
  same-key join, create/rename reservation, publication-loss cleanup, blocked
  actor deadline isolation, issued-lease retirement/restart/exhaustion,
  panic-safe replay waiters, unwind cleanup, provisional publication ownership,
  atomic ID/reservation collision handling, cleanup-timeout name retention,
  poison-aware compare removal, lock-order concurrency, and truthful concurrent
  shutdown/error collection. `local_session_ipc`
  additionally proves takeover continuation after real response loss, a
  blocked PTY does not stall status/session B, recoverable listener accept
  failures preserve live ownership, fatal listener exit rebinds the exact owned
  socket until stop is retryable, and failed bounded stop leaves the listener
  available. `local_ipc` proves byte-identical single-retry execution and that
  typed outcome unknown rotates only for a later operation.
- `session_wire` drives a pure authenticated duplex/PTY fixture through active
  snapshot acknowledgement and proves transport EOF moves the exact checkpoint,
  while explicit detach and a typed protocol failure both force the next
  reconnect to receive a full snapshot. The synchronized stream owns the one
  explicit-detach assertion; a final observation-only reconnect must remain
  unacknowledged and end by transport EOF rather than duplicate detach without
  the activation barrier.
- Concurrent shutdown evidence observes that every actor has received its end
  request before cleanup waiting, then separately requires all child PIDs and
  ownership to be released under the final absolute deadline. It must not use
  a fixed process-reap interval shorter than the platform PTY library's signal
  grace period as a proxy for concurrent initiation.
- A cleanup-timeout fixture which depends on a child-installed signal trap must
  wait for a child-authored readiness marker emitted after the trap is active.
  `PtyHost::spawn` returning proves only that the process exists; it does not
  prove that the shell has executed its setup commands. Only after readiness may
  the test inject publication failure and require `deadline_exceeded` instead
  of the original registration error.
- A `session_wire` fixture which compares history at two later attachment
  boundaries must emit a unique child marker after every terminal byte whose
  effect is asserted, including the final CRLF, and wait until that marker is
  observed before taking the first window. Seeing the preceding row text does not
  prove that a separately drained trailing control sequence has reached the
  terminal model.

## 7. Wrong vs Correct

### Wrong

```rust
let mut registry = registry.lock()?;
let result = actor.write_all(input); // may block while every session waits
registry.remove(&session_id);

runtime.model.scroll_target = Some(offset); // shared across controllers
result
```

### Correct

```rust
let actor = registry.lookup(session_id)?; // short lock only
let command = Command::input(input, absolute_deadline);
actor.try_submit(command)?;               // bounded per-session mailbox

let window = attachment.history_window(query)?; // stateless read
assert_eq!(attachment.checkpoint_revision(), previous_checkpoint);
```

Creation and cleanup additionally compare one ownership token across the name,
provisional/live actor, and session-count reservation before removing anything.

Signal-sensitive fixture ordering follows the same ownership rule:

```rust
// Wrong: the child may receive HUP before its shell installs this trap.
let child = spawn("trap '' HUP; while :; do :; done")?;
inject_publication_failure(child);

// Correct: the child emits ready only after installing the trap.
let child = spawn("trap '' HUP; emit_ready; while :; do :; done")?;
wait_for_child_ready()?;
inject_publication_failure(child);
```

Terminal-output fixture ordering must also fence the final state-changing
bytes:

```rust
// Wrong: the text can be visible before its trailing CRLF is drained.
spawn("print_history_with_final_crlf; cat")?;
wait_for_terminal_text("history-11")?;
let first_window = read_history_window()?;

// Correct: the marker is emitted only after the final asserted CRLF.
spawn("print_history_with_final_crlf; printf history-ready; cat")?;
wait_for_terminal_text("history-ready")?;
let first_window = read_history_window()?;
```

Negative mutation tests must assert fields owned by the rejected operation:

```rust
// Wrong: live PTY output may advance `revision` between these observations.
assert_eq!(before_summary, after_summary);

// Correct: rejection owns the accepted viewport, not unrelated PTY output.
assert_eq!(before_summary.session_id, after_summary.session_id);
assert_eq!(before_summary.viewport, after_summary.viewport);
```

## Forbidden patterns

- A registry in an adapter, CLI, GUI, or future Iroh transport.
- Closing a PTY because a stream, socket, tab, or attachment disappeared.
- Per-program branches for tmux, Herdr, Codex, OpenCode, or other hosted tools.
- A shared/model/Session scroll offset, a scroll target inside the remote resume
  cell, or allowing a fresh/takeover attachment to borrow an old controller's
  `ever_active` synchronization privilege.
- Retaining a history-window query/target in `SessionActor` or persisting cached
  rows in a resume checkpoint.
- Per-revision queues, transcript persistence, arbitrary launch commands, or
  Agent-specific state branches in the 1.0 terminal service.
- Holding a global replay mutex while running a side effect, retiring an
  in-flight operation, accepting a client-invented lease, or replaying one
  operation ID for a different payload.
- Awaiting a full session mailbox on the current-thread Tokio runtime, or
  reporting/flushing a successful stop while owned session resources remain.
