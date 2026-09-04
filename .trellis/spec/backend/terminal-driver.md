# Retained Terminal Driver Contract

## Scenario: PTY Drain Independent of Attachments

### 1. Scope / Trigger

This contract applies when `zterm-daemon` combines a `PtySession` with the
`zterm-terminal` host-authoritative `TerminalModel`. It exists to make terminal process lifetime
and PTY backpressure independent of Iroh connections, CLI processes, GUI tabs,
and mobile app lifecycle.

### 2. Signatures

```rust
TerminalDriver::start(
    session: PtySession,
    model: TerminalModel,
    config: TerminalDriverConfig,
) -> Result<TerminalDriver, TerminalDriverError>

TerminalDriver::attach(&self) -> TerminalAttachment
TerminalDriver::write_input(&self, bytes: &[u8]) -> Result<(), TerminalDriverError>
TerminalDriver::resize(&self, size: TerminalSize) -> Result<Revision, TerminalDriverError>
TerminalDriver::try_wait(&self) -> Result<PtyChildState, TerminalDriverError>
TerminalDriver::wait(&self) -> Result<PtyExitStatus, TerminalDriverError>
TerminalDriver::close_explicitly(&self) -> Result<PtyExitStatus, TerminalDriverError>
TerminalDriver::revision_watch(&self) -> watch::Receiver<Revision>
TerminalDriver::set_effect_target(&self, Option<AttachmentId>) -> Result<(), TerminalDriverError>
TerminalDriver::check_health(&self) -> Result<(), TerminalDriverError>
TerminalDriver::finalize_natural(self) -> Result<PtyExitStatus, TerminalDriverError>
TerminalDriver::finalize_explicit(self) -> Result<PtyExitStatus, TerminalDriverError>
TerminalDriver::interrupt_handle(&self) -> owner-only TerminalDriverInterrupt

TerminalAttachment::wait_for_revision_after(revision: Revision, timeout) -> Result<Revision, TerminalDriverError>
TerminalAttachment::sync_latest(&mut self) -> Result<TerminalSurfaceDeltaResult, TerminalDriverError>
TerminalAttachment::sync_changed(&mut self)
    -> Result<Option<TerminalSurfaceDeltaResult>, TerminalDriverError>
TerminalAttachment::discard_checkpoint(&mut self)
TerminalAttachment::latest_snapshot(&self)
    -> Result<TerminalSurfaceSnapshot, TerminalDriverError>
TerminalAttachment::history_window(query: TerminalHistoryWindowQuery)
    -> Result<TerminalSurfaceHistoryWindowResult, TerminalDriverError>
TerminalAttachment::effect_broker(&self) -> TerminalEffectBroker
```

### 3. Contracts

The retained data path is exactly:

```text
blocking PtyReader
  -> fixed-capacity, no-drop byte queue
  -> one ordered TerminalModel mutation point
  -> controlled query replies to the same PtySession
  -> validated transient host effect to one controller-targeted latest slot
  -> one latest revision condition
```

- A full byte queue blocks the reader; it never drops or overwrites PTY bytes.
- Reader EOF is the only normal transition which finishes the queue. Startup,
  model failure, and unwind abort it. `push` checks both finished and aborted
  before and after waiting for capacity, and both terminal transitions wake
  blocked producers, so no chunk can arrive after model-owner exit.
- Queue capacity and maximum pending chunks are observable and the high-water
  mark cannot exceed configured capacity.
- Attachments hold shared terminal-state access and one opaque checkpoint only.
  They never hold a PTY session, reader, writer, child, or close capability.
- Contiguous history-window projection is a stateless read-only attachment
  operation. It checks health, acquires the model mutex once, and returns the
  model-authored request-shaped Frame/Changed/Gap. The driver stores neither
  the query nor a target/baseline, and it does not advance the checkpoint or
  revision watch.
- `sync_latest` is the mandatory initial/reconnect synchronization API: a
  missing, future, or incompatible checkpoint produces a complete semantic
  snapshot. `sync_changed` is the steady-state API: an exact-equal checkpoint
  returns `None`, while a behind checkpoint returns one merged semantic delta
  or resync. Session next/final update paths must use `sync_changed` so a
  snapshot acknowledgement cannot create an `ack -> resync -> ack` loop.
- At startup the driver consumes `PtySession` into three owner-only parts: one
  reader, one `PtyIo` writer/master, and independent `PtyChild` control. A PTY
  write/flush or resize may hold only the I/O mutex; it cannot prevent the
  session owner from interrupting/reaping the child through the child mutex.
- Revision notification is latest-only. No list or channel grows once per
  revision. The Tokio watch sender overwrites one watermark; a slow attachment
  may discard its checkpoint and fetch one full latest snapshot.
- Transient host effects do not enter revision state or the PTY byte queue.
  After terminal ingest releases the model mutex, one broker mutex snapshots
  the Session-installed controller target and replaces a single pending value;
  no target means drop. A payload-free `watch<()>` only wakes attachment
  writers. `take_for(id)` removes the value only for its event-time target, and
  every target change clears stale pending content. Thus a slow or disconnected
  controller cannot backpressure PTY drain, create an effect queue, broadcast
  to observers, or replay content to a later controller.
- Zero attachments do not stop the reader, model owner, or root child.
- Dropping an attachment or transport guard changes only subscription count.
- `wait()` polls root-child state while releasing the child mutex between
  polls. Child observation/control and PTY I/O never share a mutex, so waiting
  cannot block model-generated DA/DSR/CPR replies or user input.
- Failure and latest revision share the condition variable's mutex predicate,
  so a waiter cannot miss a failure notification and misreport a deadline.
- Thread creation is ordered. Failure to start either model or reader thread
  explicitly kills/reaps the already-spawned child, aborts the queue, and
  joins every thread which did start; no child or detached partial runtime
  remains.
- Resize holds the model owner only long enough to preflight, resize the native
  PTY, mutate the model, and publish one revision. The checked model preflight
  owns size/allocation/revision validation, while the Session service owns its
  independent viewport ceiling. Native/model dimensions therefore cannot
  diverge on a predictable validation failure. There is no terminal-memory
  estimate, reservation, or rollback step.
- The driver imports model/checkpoint/error from `zterm-terminal` and terminal
  DTOs from `zterm-core`. It never exposes an Alacritty type or calls its tty,
  event-loop, or process APIs.
- Finalization consumes the driver. Natural exit waits without killing;
  explicit close uses the sole child-kill authority. Both then join the PTY
  reader and model threads after EOF/queue drain, without holding the PTY or
  terminal-model mutex across a join. Both join attempts occur even when child
  wait, the first join, or terminal health reports an error.
- `TerminalDriver::Drop` is unwind-only and never waits or joins on its caller.
  It aborts the queue, invokes the independent non-waiting child interrupt, and
  exclusively transfers reader/model handles plus owned child control to a
  background reaper. The reaper performs the truthful close/wait/joins and only
  then publishes the ownership-complete signal used by the actor registry. If
  reaper thread creation fails, the already-interrupted handles detach and the
  signal remains unreleased; normal explicit finalization remains the result
  and ownership truth authority.

No environment variable or network object participates in this data path.

### 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| byte-channel capacity is zero | `InvalidConfig` before reader transfer |
| read chunk size is zero | `InvalidConfig` before reader transfer |
| PTY reader/input/resize/wait/close fails | typed `Pty` error |
| terminal ingest/resize fails | typed `Terminal` error and queue abort |
| canonical reply bytes exceed the per-update bound | typed terminal failure and queue abort; never write a partial trusted reply stream |
| PTY read fails | `Read` recorded for attachments/waiters |
| Unix PTY slave closes and the master reports `EIO` | platform reader normalizes it to EOF |
| mutex/condvar is poisoned | `Synchronization` |
| revision/idle wait expires with no recorded failure | `Deadline` |
| revision wait races a recorded failure | return the recorded failure, not `Deadline` |
| history-window projection observes a recorded model/driver failure | return that typed failure before any Frame/Changed/Gap |
| history-window query is invalid/future, rebased, or alternate-screen | return the model-authored Gap/Rebased/Changed result; never retain a query or synthesize/merge rows in the driver |
| `sync_changed` sees checkpoint revision equal to the model | `Ok(None)`; do not replace the checkpoint or publish a frame |
| `sync_changed` sees a behind/incompatible checkpoint | one semantic Delta/Resync and replace the checkpoint at that exact latest state |
| host effect is produced with no eligible target | drop it without waking a writer or failing PTY drain |
| multiple effects precede one take | retain only the latest value for the event-time target |
| controller target changes | atomically clear the old pending value before a new target can take |

### 5. Good / Base / Bad Cases

- Good: retain the driver in the host session registry, allow attachments to
  come and go, and resync a returning consumer from the latest model.
- Good: serve independent window queries from one immutable model lock and let
  each client cache own its desired/presented offsets.
- Base: process output continuously with no attachments, then create the first
  attachment and return a full snapshot.
- Bad: put a network sender behind the PTY reader, retain one delta per
  revision, expose `PtySession` through an attachment, or hold the PTY mutex
  while waiting indefinitely for child exit.

### 6. Tests Required

- Real PTY, zero attachments: child writes more than the PTY buffer and creates
  an independent fsynced marker after all writes; child remains alive and the
  latest model contains its completion state.
- Transport ownership: drop a simulated Iroh guard and assert attachment count
  becomes zero, child remains running, and a new attachment sees later output.
- Slow attachment: pause beyond multiple bounded queue windows, discard the old
  checkpoint, resync once, and compare replayed semantic state with a separate
  latest authoritative snapshot.
- Contiguous window projection: issue independent request anchors/targets
  through two retained attachments, verify exact range and Unicode/wide/style
  rows under one model lock, and prove checkpoints, revision delivery, PTY
  state, and both callers remain unchanged.
- Synchronization: prove initial attach returns Resync, equal revision returns
  `None`, behind revision returns Delta/Resync, screen/geometry divergence
  returns Resync, and snapshot acknowledgement followed by `next_update` emits
  no empty resync loop.
- Bounded queue: assert `maximum_pending_chunks <= capacity` while processed
  chunks exceed one complete queue window.
- Query/wait regression: a raw-mode child sends DSR and exits only after
  receiving `CSI 0n`; a concurrent `TerminalDriver::wait()` must complete.
  The fixture configures its own PTY with the termios API rather than spawning
  `stty`, so the assertion measures lock ownership rather than helper-process
  scheduling or executable availability.
- Startup failure injection covers both model-thread and reader-thread creation;
  the latter proves the started model owner joins, and both prove the child PID
  is gone.
- Session isolation uses a non-reading real PTY plus an actor barrier: status,
  session B, and queued deadline expiry remain responsive while session A's
  effect is blocked; child interruption releases the real blocked writer.
- Drop/reaper coverage holds the writer mutex while the reader blocks in a real
  PTY read, proves Drop returns through a bounded observer, then proves the
  HUP-resistant child and both threads are eventually released. A queue race
  proves finished/aborted queues reject and wake a late producer.
- Broker tests linearize publish against no-controller, same-controller,
  takeover, disconnect, and reconnect target changes; prove observer
  exclusion, latest-wins, and no replay. A same-UID real-PTY fixture emits OSC
  52 through the actual model/Session/wire path and observes only the validated
  typed clipboard event.
- All waits have deadlines and fixtures remove only their own marker/process.

### 7. Wrong vs Correct

#### Wrong

```rust
// A slow socket backpressures the PTY and one entry is retained per revision.
for bytes in pty_reader {
    attachment_sender.send(parse(bytes))?;
    revision_queue.push(latest_revision);
}

attachment.model.lock()?.scroll_display(delta);
```

#### Correct

```rust
// PTY bytes always reach one model owner. Attachments query latest state.
byte_queue.push(bytes); // bounded, blocking, no-drop
let update = terminal_model.ingest(bytes)?;
latest_revision.publish(update.revision);

let window = attachment.history_window(query)?;
// `query` is not retained: cache/retry/coalescing belongs above the driver.

match attachment.sync_changed()? {
    None => return Ok(()),
    Some(update) => publish_semantic(update)?,
}
```

For polling a child behind its owner-only mutex, first copy the nonblocking
state out of a lexical guard scope, then sleep:

```rust
let child_state = {
    let mut session = pty_session.lock()?;
    session.try_wait()?
}; // guard is definitely dropped here

match child_state {
    Running => thread::sleep(POLL_INTERVAL),
    Exited(status) => return Ok(status),
}
```

## Design Decision: Latest State, Not Output Replay

Zterm reconnects to the authoritative current terminal state. Retaining a raw
output log or one delta per revision would make a disconnected phone or hidden
tab an unbounded memory/backpressure source. One opaque checkpoint per active
attachment preserves efficient warm updates while full resync remains the
single recovery path.
