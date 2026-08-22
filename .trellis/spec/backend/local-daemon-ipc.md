# Local Daemon and IPC Contract

## Scope

Apply this contract to the per-user daemon, Unix socket service, peer
credentials, detached launch, setup/status/doctor/log commands, and lifecycle
locks.

## Contracts

- One installed `zterm` executable contains a hidden internal daemon entry.
  There is one daemon per OS user and no supervisor, PID fallback, system
  service, login item, or boot registration.
- `lifecycle.lock` is short-lived launcher/setup serialization;
  `daemon.lock` is held for the daemon lifetime. The daemon never waits for the
  lifecycle lock.
- The daemon alone may remove a stale socket, and only after holding
  `daemon.lock`, observing connect failure, and validating an owned real socket.
- Linux uses `SO_PEERCRED`; macOS uses `getpeereid`. Wrong UID is rejected
  before decoding bytes. Directory/socket permissions complement but do not
  replace the credential check.
- Unary IPC uses one connection per request and the shared bounded frame codec.
  The client half-closes its write side after the frame; the server requires
  request EOF before dispatch so trailing bytes arriving in a later read are
  rejected rather than silently ignored.
  M3 read-only calls are bounded by deadline. Lifecycle stop sends and shuts
  down its response socket before signaling listener exit; a failed response
  flush leaves the daemon running, and already-stopped is idempotent at the CLI
  boundary. `OperationWindow` integration begins with M4 stateful session
  mutations, not M3 stop.
- Local readiness, status, setup validation, stop, and update preflight do not
  require Iroh, DNS, Relay, or Internet access.
- `setup` and `daemon restart` may spawn. Status, doctor, logs, daemon status,
  and daemon stop never spawn. Stop responds before shutdown and removes only
  its own socket.
- Detached spawn redirects stdio, uses a stable home cwd, and the child calls
  safe `setsid()` before runtime threads. It does not use `pre_exec` or unsafe
  code.
- This milestone reserves `local` for the future single `SessionService`, but
  contains no session registry, PTY attach, or Iroh self-dial.
- Human and JSON status are projections of one typed daemon observation.
  Running state comes from IPC; configured/stopped state may open SQLite only
  after the socket proves no `StoreActor` is live.
- Doctor validates account, committed state, and socket/lock agreement without
  spawning. Linux lifecycle output names the `systemd-logind` logout limit but
  never changes linger or installs a service.

## Required evidence

- Real same-UID request tests run on macOS/Linux; Linux CI includes a reachable
  cross-UID rejection harness.
- Multi-process tests prove concurrent launch singleflight, live/stale socket
  behavior, detach, bounded stop, restart identity preservation, and no
  spontaneous post-crash restart.
- CLI tests own the side-effect matrix and prove no inspection command creates
  state or a process.
- `single_instance` and `detached_lifecycle` are harness-free multi-process
  executables using only task-private `UserPaths`; production argv has no state
  override.
