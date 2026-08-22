# Persistent session engine and local attachment

Phase One M4 adds the terminal-session engine inside each user's daemon. It is
now possible at the service and real-socket adapter layers to create multiple
login-shell sessions, disconnect, and later attach to the same PTY and current
terminal state. The final interactive `zterm connect`/raw-terminal CLI is a
later milestone, so these interfaces are not yet exposed as public commands.

## What “persistent” means

A session belongs to the daemon, not to a socket or client process. Losing a
local attachment leaves its login shell, foreground process, working directory,
viewport, bounded recent history, and authoritative terminal model running.
Reattaching returns the same daemon-lifetime `SessionId` and a full current
snapshot before incremental updates resume.

This is deliberately not crash or reboot recovery. Explicit session close,
daemon stop/restart, upgrade, process crash, or host reboot ends the PTY. The
next daemon starts with an empty live-session registry while retaining the
installed device identity and configuration.

## Session behavior

- The first default attachment atomically creates the reserved `main` session.
  Later default attachments return to it. Closing `main` means the next default
  attachment creates a new session with a new ID.
- Named sessions are UTF-8, case-sensitive, 1–64 bytes, and cannot contain
  surrounding whitespace or control characters. Rename preserves the ID.
- Every session starts the effective OS user's interactive login shell. Its
  default working directory is that account's home. An explicit working
  directory must already be an accessible directory.
- A foreground tool exiting back to the root shell does not end the session.
  Root-shell exit does. Detach never sends a signal to the PTY.
- There is one controller in 1.0. A normal second attachment reports occupied;
  explicit takeover synchronizes the replacement first, then invalidates the
  old lease atomically. At most one pending takeover may exist, bounding server
  attachment state to the controller plus one replacement. Observer and
  simultaneous multi-controller modes remain future work.
  If a takeover response is lost, its opaque retry token lets a newly
  synchronized replacement continue that same logical operation. It cannot
  displace a controller installed by a later operation.

tmux, Herdr, Codex, OpenCode, and other terminal programs are ordinary PTY
children. Production code contains no program-specific session branch.

## Synchronization and bounds

The daemon's `TerminalModel` is the only terminal truth. Each attachment first
receives a full snapshot and must acknowledge that exact revision before input
or resize is accepted. A stale/future acknowledgement, viewport mismatch, or
inefficient delta causes `SyncRequired` followed by a replacement snapshot.
There is no unbounded output log or per-revision queue: slow clients converge
to the latest merged delta or resynchronize once.

An attachment checkpoint contains only reconstructed main/alternate visible
grids in a zero-scrollback parser. It retains `rows × columns × 2` cells no
matter how much of the bounded host history is populated; history remains owned
once by the authoritative model. Main/alternate transitions, styles, Unicode,
and resize resynchronization preserve the same latest-visible semantics.

Current resource admission is fixed:

| Resource | M4 limit |
| --- | ---: |
| Live sessions per daemon | 8 |
| Recent history per session | 2,000 rows |
| Default viewport | 120 columns × 40 rows |
| Maximum viewport | 240 columns × 80 rows |
| Aggregate fixed-cell projection | 128 MiB |
| Process RSS measurement target | 256 MiB |
| Same-UID socket connections | 32 |
| Wire frame | 8 MiB |
| Control payload | 1 MiB |

The fixed-cell projection is checked admission arithmetic, not a claim that
RSS is exactly predictable. Snapshot encoding always preserves the current
screen; when necessary it removes only the oldest complete history lines at an
ANSI reset/line boundary to remain within the frame ceiling.

## Local trust and architecture boundary

The local adapter reuses the daemon's Unix socket and authorizes the peer's
effective UID before decoding bytes (`SO_PEERCRED` on Linux, `getpeereid` on
macOS). Short session mutations remain one-frame unary connections. A terminal
attachment is one bounded duplex stream selected by its first frame.

Both paths call the same transport-independent `SessionService`. M4 does not
bind an Iroh endpoint, self-dial, pair the machine with itself, or implement
remote transport. Future authenticated QUIC and the local adapter must remain
thin callers of this same service rather than creating a second registry.

The service API is synchronous, but each `SessionActor` owns its mutable runtime
on a dedicated OS thread behind a fixed 16-command mailbox. Every command has
one absolute deadline and a queued/started/expired gate. Local socket tasks use
blocking workers and non-blocking mailbox admission, so a blocked PTY write in
session A cannot stall the current-thread Tokio runtime, status, or session B.
A queued command which expires performs no effect; a mutation which already
started continues after caller timeout/disconnect and records an exact result.

Create, rename, close, and takeover replay coordination is per operation key.
Unrelated keys execute concurrently; the same ID and semantic payload joins or
returns its exact success/typed error after response loss, while reusing an ID
for another payload is rejected. Before its first mutation, a logical local
client lazily requests a lease containing the daemon's random incarnation and a
daemon-monotonic ordinal for the same-UID principal. Readiness, status, and list
allocate none. A restart, invented/high ordinal, retired lease, or evicted
sequence returns `OperationOutcomeUnknown` without executing.

One ambiguous transport retry reuses the byte-identical encoded request and
operation ID. A typed `OperationOutcomeUnknown` response is definitive: that
mutation is not retried under a fresh lease, and only a later independent user
operation may allocate one. Leases and replay results are in memory only; there
is no automatic fresh-process recovery or generic persisted retry token.

Names use one atomic provisional/live registry slot, so create cannot race a
rename into publishing a duplicate. Session ID collision checking and resource
insertion occur in that same ordered ownership boundary, and name/resource/
actor entries carry one compare-only token. If shutdown cancels publication
after a PTY was spawned, cancellation blocks publication but keeps the original
name unavailable while the daemon closes, reaps, drains, and joins it. Until
that cleanup completes, the spawned actor remains a registry-visible
provisional owner included in shutdown. Poisoned cleanup locks are recovered
only for exact-token removal; unrelated sessions cannot be released.

Stop requests interrupt all session children concurrently. A successful stop
is acknowledged only after every child, driver thread, actor, and provisional
resource is released. If the absolute cleanup deadline expires, the daemon
returns a typed error and keeps its listener/socket available for status and a
later retry; it does not claim to have stopped while ownership remains.
Driver and actor Drop paths never join on a socket/runtime caller. They first
interrupt/abort and transfer exclusively owned thread handles to a background
reaper; the registry token remains until child and thread completion is proven.
Recoverable listener accept failures leave ownership intact and listening. On
a fatal listener exit with unreleased ownership, the daemon retains its process
and lock, compare-rebinds the exact device/inode socket it published, and keeps
status/stop retryable; it cannot remove that socket until cleanup succeeds.

On natural exit, the driver may finish and close its latest-revision watch
before the actor publishes the final drained update and typed end reason. The
local stream treats lifecycle as authoritative and stays open for
`SessionEnded`; revision-watch closure alone is not attachment cancellation.

## Focused verification

```bash
cargo test -p zterm-daemon --test session_lifecycle
cargo test -p zterm-daemon --test controller_lease
cargo test -p zterm-daemon --test session_limits
cargo test -p zterm-daemon --test session_concurrency
cargo test -p zterm-daemon --test local_session_ipc
cargo test -p zterm-daemon --test terminal_recovery
```

The explicit Foundation black-box gate additionally exercises pinned tmux and
Herdr builds in task-private sockets/directories. It is intentionally excluded
from ordinary push CI because it downloads external pinned artifacts.
