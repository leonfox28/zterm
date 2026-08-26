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

LocalRuntime::ensure_configured_daemon(&self) -> Result<DaemonReadiness, DaemonError>
LocalRuntime::pair_create(&self, ttl_seconds: u32) -> Result<PairTicketText, DaemonError>
LocalRuntime::pair_accept(&self, ticket: PairTicketText, alias: Option<&str>)
    -> Result<CommandDeviceSummary, DaemonError>
LocalRuntime::session_list(&self, target: &str)
    -> Result<Vec<CommandSessionSummary>, DaemonError>
LocalRuntime::attach(
    &self,
    target: &str,
    selector: Option<&str>,
    create_main: bool,
    takeover: bool,
    viewport: Option<TerminalSize>,
)
    -> Result<PreparedTerminalView, DaemonError>
LocalRuntime::reset_identity(&self, expected_device_id: Option<DeviceId>, force: bool)
    -> Result<IdentityResetResult, DaemonError>

run_terminal(request: TerminalRequest, runtime: &LocalRuntime)
    -> Result<(), CliError>

spawn_inside_runtime<T>(runtime: &tokio::runtime::Runtime, spawn: impl FnOnce() -> T)
    -> T
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
  bind Iroh. Public clap reaches them only through `LocalRuntime`, which owns
  committed-setup validation, singleflight daemon launch, exact target
  freezing, destructive preflight, and the safe human/JSON projections. The
  CLI never receives a socket path, `UserPaths`, store, identity key, Endpoint,
  route, or operation lease.
- Public clap exposes setup/status/doctor/logs, pair create/accept, device
  list/rename/revoke, connect, Session list/new/attach/rename/close, daemon
  status/stop/restart, and `reset --identity`. Bare invocation observes first:
  not-setup prints fixed setup guidance without creating state; configured
  invocation is exactly local `main` create-or-attach. Help, version, parsing
  failures, status, doctor, logs, daemon status, and daemon stop never spawn.
  Setup and restart explicitly spawn; configured pair/device/connect/Session
  commands may singleflight-start one daemon but never perform setup.
- Pair accept has no ticket positional argument, flag, or environment input.
  Its default owner is a no-echo TTY line reader; non-TTY input is rejected
  before reading unless `--stdin` explicitly selects the 16 KiB-bounded EOF
  reader. Both paths transfer immediately into `PairTicketText`/zeroizing
  owners. Pair create writes the ticket once to stdout; Debug and error
  projections remain redacted.
- User target resolution accepts reserved `local`, one exact case-sensitive
  outbound alias, or one canonical 64-lowercase-hex DeviceId. Exact aliases
  precede rejection of hex-looking short/prefix text; a full DeviceId candidate
  must be lowercase and a full-ID/exact-alias collision is ambiguous.
  Session selectors are exact names or canonical 32-lowercase-hex SessionIds.
  A default `connect` uses atomic `create_main`; after setup, bare invocation
  resolves to the same path. Ordinary attach never steals a controller;
  explicit takeover is the only CLI request for replacement.
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
- `LocalSessionUnaryRequest` is an ordinary 1 MiB-bounded control payload which
  contains exactly one allowed preencoded Session unary frame. The daemon
  validates its frozen full target and correlation without using the payload as
  a second codec or retry source. A remote mutation outer envelope is never
  replayed after a full or partial Unix write: missing EOF, malformed framing,
  wrong ID/kind, or invalid typed payload immediately becomes
  `operation_outcome_unknown`. Only a read-only outer request may retry once;
  stateful lease allocation uses one outer attempt and one remote service-stream
  attempt, returning its typed post-write failure without allocating again. The
  daemon-owned remote client alone owns the one possible Iroh mutation retry.
- A remote-target `TerminalAttachRequest` creates one daemon-owned desired-view
  bridge behind the same-UID duplex connection. The bridge keeps one stable
  local attachment ID and one `ConnectionDemand` for the view lifetime, while
  each remote stream receives a fresh host attachment ID. It emits local-only
  `Preparing`, `Synchronizing`, `Active`, and `Reconnecting` events, rewrites
  attachment IDs at the boundary, drops input while non-active, and retains
  only the latest validated viewport. Every open, initial exchange, full-sync
  exchange, remote write, local write, detach, and control forward is bounded by
  an existing absolute attempt/operation deadline; active reads remain
  long-lived and local EOF/detach remains authoritative. If a post-`Active`
  replacement attach for the frozen SessionId races the old host reader's EOF
  and receives `SessionOccupied`, the bridge closes that rejected epoch and
  waits a fixed cancellable 250 ms while continuing the same input-drop and
  latest-viewport rules. A first-ever `SessionOccupied` remains terminal.
- A local or remote initial attachment has one deadline covering every
  pre-snapshot transport-state frame through the complete correlated snapshot
  or typed service error. For `create_main`, encode/connect failures before the
  request write remain definitive; any full or partial write followed by an
  unvalidated timeout, transport close, malformed response, or correlation
  failure is `operation_outcome_unknown`. A complete correlated service error
  remains definitive. An existing-session attach retains its exact bounded
  transport/protocol failure because it has no create side effect.
- The high-level raw-terminal owner validates stdin/stdout before starting any
  attachment work. Once `session new` or `create_main` may have submitted its
  stateful request, local detach, stdin EOF, and SIGINT/SIGTERM/SIGHUP record
  cancellation but continue polling the same owned future to its exact bounded
  result. Exact success reports the stable SessionId and detaches only the
  view; create-then-attach failure preserves `CreatedSessionAttach`; an
  unprovable post-submit result remains `operation_outcome_unknown`.
- Each non-`Active` terminal transition advances the input epoch and clears the
  prefix. Returning to `Active` first joins the old stdin reader, flushes queued
  kernel input, advances the epoch, installs the replacement reader, and only
  then accepts input. `SIGWINCH` may initiate another sync, so viewport
  publication or a resize acknowledgement alone is not an input fence. Pure UI
  tests own the exact reader-replacement ordering. The multiprocess PTY fixture
  uses the production `run_terminal` entry and bounded idempotent shell probes;
  it must not add renderer markers or test branches to the product loop.
- A bridge retains at most eight correlated lease/takeover controls. Epoch loss
  completes every sent lease request with its original typed transport failure
  and every sent takeover with `operation_outcome_unknown`; neither is replayed.
  A correlated ordinary `ServiceError` preserves its typed code and request
  correlation but discards the untrusted peer message, re-encodes one stable
  content-free local detail, and removes only its pending cell. An uncorrelated
  response or fatal authorization, wire,
  protocol, Session, or lease outcome terminates the view. A fatal bridge error
  is flushed as one typed local service error before the duplex connection
  closes.
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
- Daemon stop/restart with active Sessions requires explicit `--force`; the
  public surface has no implicit interactive bypass. Close, revoke, and
  identity reset instead use exact preflight plus interactive `yes` or
  noninteractive `--yes`; identity reset additionally requires `--force` when
  Sessions are active. Reset performs a bounded stop, acquires
  `lifecycle.lock`, rechecks stopped ownership and the confirmed identity, and
  removes only the validated managed state root. It sends no `RevokeSelf`,
  does not remove the binary, and does not run setup.
- Detached spawn redirects stdio, uses a stable home cwd, and the child calls
  safe `setsid()` before runtime threads. It does not use `pre_exec` or unsafe
  code.
- The detached daemon composes synchronous owners before building its
  current-thread Tokio runtime. Every daemon-owned `tokio::spawn` performed
  from that synchronous path must run inside this exact runtime's `enter()`
  guard. The guard covers only task creation and is released before the
  listener loop; subsequent `runtime.block_on` calls drive the bound tasks.
  Production startup must never rely on an ambient runtime inherited from the
  launcher or from a `#[tokio::test]`.
- Local session and terminal calls use the single transport-independent
  `SessionService`; they never pair, resolve an alias, bind Iroh, or self-dial.
  `LocalAttachmentClient` is a daemon-internal/test-facing real socket adapter,
  retained below the public raw-terminal UI; the CLI never owns its socket,
  decoder, operation lease, route, or remote transport.
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
| ambiguous same-UID loss on a local-target mutation | retry once with identical bytes/ID/deadline |
| remote mutation outer envelope was partially/fully written but has no fully validated correlated response | `operation_outcome_unknown`; do not reconnect or replay the envelope |
| read-only remote outer envelope has a post-write failure | retry once with identical envelope bytes and the same deadline |
| remote attachment stream is lost with a pending lease / takeover | original typed transport failure / `operation_outcome_unknown`; remove the pending cell and never replay it |
| `create_main` request was written but no complete correlated initial result is validated | `operation_outcome_unknown`; do not claim the default Session was absent or retry under a new identity |
| existing-session initial attach receives states but no snapshot before its total deadline | `deadline_exceeded`; close only that view |
| post-active frozen-session attach receives `session_occupied` while the old host reader is half-open | close the rejected epoch, remain reconnecting, drain/drop input and coalesce viewport for 250 ms, then retry the same SessionId without `create_main` |
| first-ever attach receives `session_occupied` | flush the typed terminal error and close the local view; do not retry |
| active remote stream returns a correlated ordinary service error | forward it, remove the pending cell, and keep the attachment alive |
| remote attachment frame is malformed or fatally unauthorized/incompatible | flush a typed local service error, then close only that local view |
| definitive outcome unknown | do not retry that mutation under a new lease |
| a daemon-requiring public command observes no committed setup | `not_setup` with `zterm setup` guidance; do not create identity or state |
| pair accept uses a non-TTY without `--stdin` | usage error before reading ticket bytes or starting ticket parsing |
| PTY test infers input readiness from viewport publication alone | invalid evidence; use the pure input-fence test or a bounded idempotent fixture probe |
| target is short/prefix/uppercase ID, ambiguous full ID/alias, self ID without `local`, or inbound-only for Session access | exact selector/direction error; do not acquire remote demand |
| ordinary attach finds a controller | `session_occupied`; do not input, resize, detach, or replace it |
| daemon stop/restart or identity reset would end Sessions without `--force` | refuse before stop; report only the safe Session count/impact |
| close/revoke/reset is noninteractive without `--yes` | refuse before mutation |
| stop cleanup or response flush fails | keep listener/socket and ownership available for status/retry |
| fatal accept while a child remains owned | exact-token rebind under held daemon lock; resume service |
| socket path was replaced after bind | never unlink or overwrite the replacement |
| synchronous daemon composition calls `tokio::spawn` outside its owned runtime | invalid startup boundary; the pure runtime regression fails deterministically before release |

## 5. Good / Base / Bad Cases

- **Good:** authenticate peer credentials, decode one shared frame, dispatch to
  `SessionService` through `spawn_blocking`, and flush a terminal error before
  closing only the offending attachment.
- **Base:** status/doctor/logs observe a stopped or running daemon without
  creating paths, allocating mutation leases, or starting a process.
- **Good:** parse one public target, let `LocalRuntime` freeze it to `local` or
  a full DeviceId, and pass only the typed request/view to the CLI renderer.
- **Good:** prove shell-ready and eventual interactive echo through the
  production terminal entry; exercise resize plus signal restoration separately
  from prefix detach, while pure tests own the exact Active input fence.
- **Bad:** trust socket permissions without peer credentials, decode before the
  UID gate, let clap accept a ticket/path/socket override, block the
  current-thread runtime on PTY work, or remove a socket by pathname without
  comparing the listener's device/inode/change-time token. It is also invalid
  to infer Active from independent resize/viewport notifications.
- **Good:** enter the daemon-owned runtime only while spawning an async owner,
  release the guard, then let the existing listener `block_on` drive it.
- **Bad:** call a production spawn seam from synchronous startup merely because
  all existing tests happen to run inside `#[tokio::test]`.

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
- A pure synchronous lifecycle unit builds the same current-thread runtime,
  spawns through `spawn_inside_runtime`, and joins the task with that runtime.
  Removing the `enter()` guard must reproduce Tokio's no-reactor failure. The
  companion network lifecycle test injects every failure before Endpoint bind;
  neither test may open UDP, perform DNS, or contact a Relay.
- CLI tests own the complete help/side-effect matrix: the public tree has no
  state/identity/socket/ticket override; bare before/after setup, help/version,
  parse errors, every inspection command, daemon stop/restart, and each
  daemon-requiring command assert their exact create/spawn behavior. Pair tests
  prove no-echo restoration and zeroized/redacted success/error/panic paths;
  reset tests prove exact confirmation, active-Session force, no-follow fixed
  inventory, retryable partial deletion, and no implicit setup.
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
  next independent mutation. It also proves a remote mutation outer envelope
  is sent exactly once for malformed/truncated, wrong-ID/kind, and invalid
  typed replies while read-only outer requests retain one byte-identical retry.
  `local_session_ipc` proves a recoverable injected accept failure preserves the
  listener and its live session, and an injected fatal accept in the actual
  `run_daemon` listener loop rebinds while a HUP-resistant child remains owned,
  then accepts a truthful stop retry.
- `remote_attachment` proves stable-local/fresh-remote ID mapping, bounded
  reconnect cancellation and writes, snapshot-first input gating, viewport
  coalescing, paused-time half-open occupancy retry, first-ever occupancy
  termination, correlated control completion, other terminal error projection,
  and state-event ordering over pure fake streams. The local attachment client
  separately proves routing and validated transport-state consumption over a
  real same-UID Unix duplex connection.
- The CLI multiprocess PTY gate uses a task-private deterministic shell. The
  connect child proves ready -> eventual interactive echo -> default detach
  through the unmodified `run_terminal`; the bare child separately proves
  SIGWINCH revision/viewport, SIGTERM cancellation, termios restoration,
  bounded reap, and panic cleanup. Stress it sequentially and concurrently,
  then assert no fixture daemon is orphaned. No diagnostic may contain
  terminal/cwd bytes.

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

```rust
// Wrong: constructing a runtime does not make it ambient on this thread.
let runtime = tokio::runtime::Builder::new_current_thread().build()?;
let supervisor = startup.spawn(handle); // tokio::spawn panics: no reactor

// Correct: bind task creation to the owned runtime, then release the guard.
let supervisor = spawn_inside_runtime(&runtime, || startup.spawn(handle));
runtime.block_on(serve_local(...))?;
```

The duplex branch retains the same decoder leftovers and uses bounded control
state plus latest-only watches instead of a per-revision queue.

A viewport observation is not an Active input fence. Do not send a resize and
then immediately inject a one-shot input or detach. Keep exact reader-fence
ordering in pure tests; a process fixture may use bounded idempotent readiness
probes and must keep resize/signal restoration in a separate deterministic
phase when their synchronization can race input.

## Forbidden patterns

- A second frame decoder, session registry, replay engine, or peer-auth policy in
  the CLI or remote adapter.
- Calling a blocking `SessionService`/PTY operation inline on the current-thread
  Tokio runtime.
- Calling `tokio::spawn` from synchronous daemon startup outside the exact
  daemon-owned runtime's `enter()` guard.
- Removing or rebinding a socket without the held daemon lock and exact
  device/inode/change-time ownership token.
- Reporting successful stop before every registry-owned child/thread/reservation
  is released.
