# Retained Terminal Driver Contract

## Scenario: PTY Drain Independent of Attachments

### 1. Scope / Trigger

This contract applies when `zterm-daemon` combines a `PtySession` with the
host-authoritative `TerminalModel`. It exists to make terminal process lifetime
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
TerminalDriver::resize(&self, size: TerminalSize) -> Result<u64, TerminalDriverError>
TerminalDriver::try_wait(&self) -> Result<PtyChildState, TerminalDriverError>
TerminalDriver::wait(&self) -> Result<PtyExitStatus, TerminalDriverError>
TerminalDriver::close_explicitly(&self) -> Result<PtyExitStatus, TerminalDriverError>

TerminalAttachment::wait_for_revision_after(revision, timeout) -> Result<u64, TerminalDriverError>
TerminalAttachment::sync_latest(&mut self) -> Result<TerminalDeltaResult, TerminalDriverError>
TerminalAttachment::discard_checkpoint(&mut self)
TerminalAttachment::latest_snapshot(&self) -> Result<TerminalSnapshot, TerminalDriverError>
```

### 3. Contracts

The retained data path is exactly:

```text
blocking PtyReader
  -> fixed-capacity, no-drop byte queue
  -> one ordered TerminalModel mutation point
  -> controlled query replies to the same PtySession
  -> one latest revision condition
```

- A full byte queue blocks the reader; it never drops or overwrites PTY bytes.
- Queue capacity and maximum pending chunks are observable and the high-water
  mark cannot exceed configured capacity.
- Attachments hold shared terminal-state access and one opaque checkpoint only.
  They never hold a PTY session, reader, writer, child, or close capability.
- Revision notification is latest-only. No list or channel grows once per
  revision. A slow attachment may discard its checkpoint and fetch one full
  latest snapshot.
- Zero attachments do not stop the reader, model owner, or root child.
- Dropping an attachment or transport guard changes only subscription count.
- `wait()` must poll root-child state while releasing the PTY mutex between
  polls. The mutex guard must end in an explicit lexical scope before any
  sleep, yield, retry, channel wait, or other blocking operation; do not rely
  on a temporary guard in a `match`/`if let` scrutinee because its lifetime can
  extend through the selected branch. `wait()` must not block model-generated
  DA/DSR/CPR replies or user input.
- Failure and latest revision share the condition variable's mutex predicate,
  so a waiter cannot miss a failure notification and misreport a deadline.
- Thread creation is ordered so a later spawn failure can finish the queue and
  join the already-created model thread; no detached partial runtime remains.

No environment variable or network object participates in this data path.

### 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| byte-channel capacity is zero | `InvalidConfig` before reader transfer |
| read chunk size is zero | `InvalidConfig` before reader transfer |
| PTY reader/input/resize/wait/close fails | typed `Pty` error |
| terminal ingest/resize fails | typed `Terminal` error and queue abort |
| PTY read fails | `Read` recorded for attachments/waiters |
| mutex/condvar is poisoned | `Synchronization` |
| revision/idle wait expires with no recorded failure | `Deadline` |
| revision wait races a recorded failure | return the recorded failure, not `Deadline` |

### 5. Good / Base / Bad Cases

- Good: retain the driver in the host session registry, allow attachments to
  come and go, and resync a returning consumer from the latest model.
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
- Bounded queue: assert `maximum_pending_chunks <= capacity` while processed
  chunks exceed one complete queue window.
- Query/wait regression: a raw-mode child sends DSR and exits only after
  receiving `CSI 0n`; a concurrent `TerminalDriver::wait()` must complete.
  The fixture configures its own PTY with the termios API rather than spawning
  `stty`, so the assertion measures lock ownership rather than helper-process
  scheduling or executable availability.
- All waits have deadlines and fixtures remove only their own marker/process.

### 7. Wrong vs Correct

#### Wrong

```rust
// A slow socket backpressures the PTY and one entry is retained per revision.
for bytes in pty_reader {
    attachment_sender.send(parse(bytes))?;
    revision_queue.push(latest_revision);
}
```

#### Correct

```rust
// PTY bytes always reach one model owner. Attachments query latest state.
byte_queue.push(bytes); // bounded, blocking, no-drop
let update = terminal_model.ingest(bytes)?;
latest_revision.publish(update.revision);
```

For polling a child behind a shared PTY mutex, first copy the nonblocking state
out of a lexical guard scope, then sleep:

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
