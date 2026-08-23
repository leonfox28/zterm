# Local Daemon and IPC Contract

## 1. Scope / Trigger

Apply this contract to the per-user daemon, Unix socket service, peer
credentials, detached launch, setup/status/doctor/log commands, and lifecycle
locks.

## 2. Signatures

```rust
LocalClient::new(socket: impl Into<PathBuf>) -> LocalClient
LocalClient::readiness(&self) -> Result<DaemonReadiness, DaemonError>
LocalClient::status(&self) -> Result<DaemonStatus, DaemonError>
LocalClient::list_sessions(&self) -> Result<Vec<SessionSummary>, DaemonError>
LocalClient::create_session(&self, name, cwd, viewport)
    -> Result<SessionSummary, DaemonError>
LocalClient::rename_session(&self, session_id, name)
    -> Result<SessionSummary, DaemonError>
LocalClient::stop(&self, force: bool) -> Result<SessionImpact, DaemonError>

LocalPairingClient::create(&self, ttl_seconds: u32)
    -> Result<PairTicketText, DaemonError>
LocalPairingClient::accept(&self, ticket: PairTicketText, alias: Option<&DeviceAlias>)
    -> Result<DeviceSummary, DaemonError>
LocalDeviceClient::list(&self) -> Result<Vec<DeviceSummary>, DaemonError>
LocalDeviceClient::rename(&self, device_id: DeviceId, alias: &DeviceAlias)
    -> Result<DeviceSummary, DaemonError>
LocalDeviceClient::revoke(&self, device_id: DeviceId)
    -> Result<DeviceSummary, DaemonError>
```

Unary IPC is `varint length + WireFrame + write-half EOF -> one response`.
`TerminalAttachRequest` selects the duplex stream; lease allocation is the
strict unary `SessionOperationLeaseRequest -> SessionOperationLeaseResponse`.

## 3. Contracts

- One installed `zterm` executable contains a hidden internal daemon entry.
  There is one daemon per OS user and no supervisor, PID fallback, system
  service, login item, or boot registration.
- `lifecycle.lock` is short-lived launcher/setup serialization;
  `daemon.lock` is held for the daemon lifetime. The daemon never waits for the
  lifecycle lock.
- The daemon alone may remove a stale socket, and only after holding
  `daemon.lock`, observing connect failure, and validating an owned real socket.
  Each bound listener also carries the socket path's device/inode/change-time
  token (Linux may immediately reuse an unlinked inode);
  fatal-listener rebind and final removal compare that exact token and refuse to
  unlink a same-UID path which was replaced after publication.
- Linux uses `SO_PEERCRED`; macOS uses `getpeereid`. Wrong UID is rejected
  before decoding bytes. Directory/socket permissions complement but do not
  replace the credential check.
- IPC uses one shared bounded frame codec and classifies a connection from its
  first decoded frame. Unary calls use one connection per request.
  The client half-closes its write side after the frame; the server requires
  request EOF before dispatch so trailing bytes arriving in a later read are
  rejected rather than silently ignored. A `TerminalAttachRequest` instead
  enters one long-lived duplex stream and preserves decoder state plus any
  complete frames received in the same read.
  All session calls carry one absolute deadline. Potentially blocking
  synchronous service/attachment work runs under `spawn_blocking`; the
  current-thread Tokio runtime never waits inline for a full actor mailbox or
  a PTY effect. Timing out drops only the waiter—an already-started mutation
  continues and records its exact replay result.
- Local pair/device kinds 12-21 use the same credential gate, decoder, strict
  unary EOF, and typed service-error response. Pair dispatch remains async;
  blocking Store/Directory/device projection work runs off the socket runtime.
  Sensitive first frames are moved rather than cloned, and ticket/request/reply
  buffers are zeroized after the one byte-identical retry window.
- `LocalPairingClient` and `LocalDeviceClient` are doc-hidden daemon-internal /
  test adapters. They never spawn a daemon, open SQLite, read `identity.key`, or
  bind Iroh. Public clap does not expose them before the M8 security/UX task.
- `SessionOperationLeaseRequest/Response` is the mutation-only control exchange
  for a daemon-issued lease. A logical `LocalClient` requests it lazily before
  its first mutation and caches it; readiness, status, and session list do not
  allocate a lease or write replay state. Request IDs, lease ordinals, and
  operation sequences fail explicitly at exhaustion and never wrap.
- A mutation is encoded once. Only an ambiguous transport failure may trigger
  one retry, using byte-identical bytes, request ID, operation ID, payload, and
  the same absolute deadline. A complete response, including typed
  `operation_outcome_unknown`, is definitive. Outcome unknown poisons the
  cached lease; that logical mutation is not retried under a new lease, while a
  later independent operation may request one.
- Lifecycle stop first performs bounded concurrent session cleanup. Only full
  ownership release may produce a successful stop response; that response is
  flushed and its socket shut down before listener exit is signaled. A cleanup
  deadline, failed response flush, or dropped caller leaves the listener and
  owned socket running for status and retry. Already-stopped remains
  idempotent at the CLI boundary.
- Interrupted/would-block/connection-aborted/reset/timed-out accept errors stay
  inside the listener loop and do not drop live session ownership. A fatal
  accept error aborts only its connection handlers and returns to the
  `run_daemon` ownership loop. That loop first attempts bounded concurrent
  session cleanup. If any owner remains, it keeps the process, daemon lock,
  store, service, and child registry alive, compare-rebinds its exact socket
  token with bounded backoff, and resumes status/stop service. It exits and
  removes the exact socket only after all ownership is released.
- M4 session list/create/rename/close remain strict unary calls. The attachment
  reader accepts only snapshot acknowledgement, sync, input, resize, detach,
  and takeover for the bound attachment. A protocol error flushes one typed
  error before closing only that stream.
- Attachment output uses one fixed-capacity control queue plus latest-only
  revision/lifecycle watches. A slow socket writer cannot backpressure the PTY
  reader or accumulate one message per terminal revision.
  Natural finalization may close the driver revision watch before the actor
  publishes its final drained update and `SessionEnded`. The socket writer
  disables revision polling on that expected closure (or a racing
  `SessionNotFound`) and keeps the stream alive for the lifecycle channel,
  which is the authority for terminal termination.
- Local readiness, status, setup validation, stop, and update preflight do not
  require Iroh, DNS, Relay, or Internet access.
- `setup` and `daemon restart` may spawn. Status, doctor, logs, daemon status,
  and daemon stop never spawn a daemon. A successful stop responds after
  session shutdown and removes only its own socket during the normal daemon
  lifecycle handoff.
- Detached spawn redirects stdio, uses a stable home cwd, and the child calls
  safe `setsid()` before runtime threads. It does not use `pre_exec` or unsafe
  code.
- Local session and terminal calls use the single transport-independent
  `SessionService`; they never pair, resolve an alias, bind Iroh, or self-dial.
  `LocalAttachmentClient` is a daemon-internal/test-facing real socket adapter,
  not the final raw-terminal UI.
- Human and JSON status are projections of one typed daemon observation.
  Running state comes from IPC; configured/stopped state may open SQLite only
  after the socket proves no `StoreActor` is live.
- Doctor validates account, committed state, and socket/lock agreement without
  spawning. Linux lifecycle output names the `systemd-logind` logout limit but
  never changes linger or installs a service.

## 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| peer effective UID differs from daemon owner | close with zero response bytes before frame decode |
| unary request has trailing bytes, missing EOF, malformed frame, or excessive size | typed/connection-local protocol failure; listener remains usable |
| request deadline expires before dispatch | `deadline_exceeded`, no effect begins |
| request times out after actor start | drop only waiter; accepted effect completes into replay state |
| ambiguous transport loss on a mutation | retry once with identical bytes/ID/deadline |
| definitive outcome unknown | do not retry that mutation under a new lease |
| stop cleanup or response flush fails | keep listener/socket and ownership available for status/retry |
| fatal accept while a child remains owned | exact-token rebind under held daemon lock; resume service |
| socket path was replaced after bind | never unlink or overwrite the replacement |

## 5. Good / Base / Bad Cases

- **Good:** authenticate peer credentials, decode one shared frame, dispatch to
  `SessionService` through `spawn_blocking`, and flush a terminal error before
  closing only the offending attachment.
- **Base:** status/doctor/logs observe a stopped or running daemon without
  creating paths, allocating mutation leases, or starting a process.
- **Bad:** trust socket permissions without peer credentials, decode before the
  UID gate, block the current-thread runtime on PTY work, or remove a socket by
  pathname without comparing the listener's device/inode/change-time token.

## 6. Tests Required

- Real same-UID unary and duplex tests run on macOS/Linux; Linux CI includes a reachable
  cross-UID rejection harness. A helper executed as the foreign UID must live
  below one test-private directory whose parents are searchable by that UID;
  the copied executable is execute-only, `sudo` is noninteractive, and the test
  requires both zero response bytes for the rejected peer and a successful
  owner request afterward. Running a binary directly from a CI workspace whose
  parent directories are not searchable is fixture failure, not peer-gate
  evidence.
- Multi-process tests prove concurrent launch singleflight, live/stale socket
  behavior, detach, bounded stop, restart identity preservation, and no
  spontaneous post-crash restart.
- CLI tests own the side-effect matrix and prove no inspection command creates
  state or a process.
- `single_instance` and `detached_lifecycle` are harness-free multi-process
  executables using only task-private `UserPaths`; production argv has no state
  override.
- `local_session_ipc` proves session mutations, detach/reconnect, and daemon-stop
  events; it also drops a create response and retries the same operation ID on
  a new socket, blocks a real session-A PTY writer while status/session B
  progress, proves final output and the typed natural-exit event survive the
  revision-watch-close race, and proves a failed bounded stop keeps the listener
  available until ownership is released. `terminal_recovery` proves
  resynchronization and that an invalid attachment kind does not poison other
  sessions or the listener.
- `local_ipc` additionally proves a dropped mutation response is retried once
  with byte-identical request bytes and one server execution, and that a typed
  outcome-unknown response is not retried and rotates the lease only on the
  next independent mutation. `local_session_ipc` proves a recoverable injected
  accept failure preserves the listener and its live session, and an injected
  fatal accept in the actual `run_daemon` listener loop rebinds while a
  HUP-resistant child remains owned, then accepts a truthful stop retry.

## 7. Wrong vs Correct

### Wrong

```rust
let request = decode(stream.read().await?)?; // peer not authenticated
service.dispatch(request);                   // may block Tokio inline
remove_file(socket_path)?;                   // pathname may be replaced
```

### Correct

```rust
verify_same_uid(&stream)?;
let request = read_one_strict_frame_and_eof(&mut stream).await?;
let reply = spawn_blocking(move || service.dispatch(request)).await??;
remove_socket_only_if_token_matches(socket_path, listener_token)?;
```

The duplex branch retains the same decoder leftovers and uses bounded control
state plus latest-only watches instead of a per-revision queue.

## Forbidden patterns

- A second frame decoder, session registry, replay engine, or peer-auth policy in
  the CLI or a future remote adapter.
- Calling a blocking `SessionService`/PTY operation inline on the current-thread
  Tokio runtime.
- Removing or rebinding a socket without the held daemon lock and exact
  device/inode/change-time ownership token.
- Reporting successful stop before every registry-owned child/thread/reservation
  is released.
