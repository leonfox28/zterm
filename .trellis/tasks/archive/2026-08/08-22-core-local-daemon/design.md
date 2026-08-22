# Core、本地 daemon 与 IPC 技术设计

## 1. Scope

本设计只覆盖父任务 M2–M3：

1. 共享 core/protobuf 契约；
2. effective-user 路径、权限、原子文件、identity/config/SQLite；
3. 每用户唯一 daemon、same-UID Unix IPC、按需 detached-spawn；
4. setup/status/doctor/logs/daemon lifecycle CLI。

Foundation 的 PTY/VT/TerminalDriver 和官方 Iroh N0 profile 继续保留。M4 才实现唯一的 `SessionService`/registry；M5–M6 才 bind Iroh endpoint 并实现 pairing/auth。当前任务不能通过临时 session map 或 self-Iroh connection 冒充这些后续里程碑。

## 2. Architecture and data flow

```text
                        one installed executable
┌──────────────────────────────── zterm ────────────────────────────────┐
│ clap/user commands                         hidden internal daemon mode │
│                                                                      │
│ setup ──> daemon-owned bootstrap library ──> identity/config/store    │
│   │                                                                  │
│   └──> ensure_daemon ── spawn lock ──> detached child ── setsid       │
│                                                                      │
│ status/stop ─> LocalClient ─> Unix socket ─> peer UID gate            │
│                                           └> frame decoder            │
│                                           └> DaemonService            │
│                                                ├> StoreActor          │
│                                                └> lifecycle state      │
└──────────────────────────────────────────────────────────────────────┘

future M4:
LocalClient target=local ──> same DaemonService ──> one SessionService

future M5:
remote adapter/Iroh ─────────────────────────────> same SessionService
```

### Setup flow

```text
resolve effective account
  -> derive UserPaths
  -> acquire short-lived lifecycle.lock
  -> if daemon is live: validate through IPC and return
  -> validate/create ~/.zterm and managed nodes
  -> load existing key or atomically create one
  -> open/migrate state DB and verify public identity metadata
  -> load existing config or atomically create it
  -> release lifecycle.lock
  -> ensure_daemon
  -> readiness RPC
```

### Unary local request flow

```text
connect socket
  -> server reads OS peer credentials
  -> reject wrong UID before protobuf dispatch
  -> client half-closes its write side after one request
  -> bounded frame decode and request EOF/trailing-byte check
  -> validate wire major/kind/deadline/domain fields once
  -> dispatch one request
  -> return one typed result
  -> close connection
```

One connection per unary operation makes cancellation structural: closing it cancels work not yet committed. M4 terminal views may keep a connection open, but reuse the same peer gate/frame codec and still route to the one SessionService.

## 3. Layer ownership

| Layer | Owns | Must not own |
| --- | --- | --- |
| `zterm-core` | IDs, principals, revision, capability/resource values, operation replay state, domain errors | prost, rusqlite, Iroh endpoint, OS paths |
| `zterm-proto` | `.proto` generation, frame codec, numeric kind registry, proto↔domain conversion/limits | socket, database, CLI rendering |
| `zterm-platform` | effective account, UserPaths, mode/owner/symlink checks, atomic files, file-lock guard, runtime socket path, peer UID, child stdio/detach primitives, existing PTY | config semantics, SQL, protobuf dispatch |
| `zterm-daemon` | config/identity bootstrap, StateStore/StoreActor, DaemonService, local IPC server/client, lifecycle orchestration, infrastructure adapter | terminal rendering UX, direct SQL from CLI |
| `zterm-cli` | clap parsing, prompt/confirmation, human/JSON formatting, current-exe child invocation | secret-key parsing, SQL, second daemon/session state |

`zterm-platform::pty` and new user-path code share one public `EffectiveAccount`; the PTY module no longer keeps a private duplicate account record.

## 4. Core domain contract

### 4.1 Identifiers

```rust
DeviceId([u8; 32])
SessionId([u8; 16])
AttachmentId([u8; 16])
Revision(u64)
OperationId { client_epoch: u64, sequence: u64 }
```

Core exposes checked `from_bytes`/byte projections. Human EndpointId formatting stays in the Iroh adapter instead of recreating Iroh encoding in core. Session/Attachment random generation belongs to M4; M2 only freezes representation.

`AttachmentPrincipal`:

```rust
RemoteEndpoint {
    device_id: DeviceId,
    auth_generation: u64,
}
LocalSameUid {
    own_device_id: DeviceId,
    local_view_id: AttachmentId,
}
```

The local variant is constructed only after platform peer-UID verification. It does not query `device_auth`.

`ControllerLease` contains the controlling `AttachmentId` and monotonic lease generation. M4 owns transitions; M2 supplies the shared type.

### 4.2 Capability and resource values

`Capabilities` is a zterm-owned `u64` newtype with named bits. M2 reserves normal terminal/session/local lifecycle bits and future `DEVICE_EVENTS`, `HISTORY_PAGING`, `AGENT_EVENTS`; unrecognized bits are retained/ignored according to handshake negotiation rather than becoming Rust enum decode failures.

Default `ResourceLimits`:

| Value | Default |
| --- | ---: |
| max live sessions | 8 |
| recent history rows/session | 2,000 |
| no-controller viewport | 120×40 |
| maximum accepted viewport | 240×80 |
| aggregate fixed-cell projection | 128 MiB |
| complete daemon RSS target (measurement, not admission field) | 256 MiB |
| local unary connections | 32 |
| local request deadline ceiling | 30 s |

M4 enforces terminal/session limits. M3 only enforces connection/deadline limits.

### 4.3 Operation replay window

`OperationWindow<R>` is created for one authenticated principal and one fixed `client_epoch`; transport epoch rotation policy remains M5. It owns a bounded ordered map from sequence to the exact result, including typed failures.

```text
epoch mismatch                    -> OutcomeUnknown
sequence retained                 -> Replay(original result)
sequence < low_water and missing  -> OutcomeUnknown
otherwise                         -> execute closure exactly once, retain result
capacity exceeded                 -> evict lowest sequence, advance low_water
```

The API executes through one serialized closure, so a duplicate cannot observe a half-committed pending entry. Store/Session actors remain the serialization owner. The window is in memory and disappears with daemon state; no session operation survives daemon restart.

M2 freezes and tests this state machine. Actual replay integration begins in
M4, where stateful `SessionService` create/rename/close/takeover mutations have
a committed result worth replaying. M3 lifecycle stop deliberately does not
create an in-memory replay window: it signals daemon shutdown only after its
response is flushed, and an already-stopped CLI request is naturally
idempotent.

## 5. Protobuf and frame contract

### 5.1 Source files

```text
proto/zterm/v1/common.proto
proto/zterm/v1/wire.proto
proto/zterm/v1/local.proto
proto/zterm/v1/pairing.proto
proto/zterm/v1/session.proto
proto/zterm/v1/terminal.proto
```

- `common.proto`: byte IDs, OperationId, versions, capabilities, target selector, typed error.
- `wire.proto`: `WireFrame { wire_major, kind, payload }` and stable numeric kinds.
- `local.proto`: readiness/status/stop/update-preflight unary request/result.
- `pairing.proto`: versioned ticket/offer/handshake shapes only; no handler in M3.
- `session.proto`: list/create/rename/close/takeover metadata and request/result shapes only.
- `terminal.proto`: attach, snapshot/delta, input/resize/detach/sync shapes based on the Foundation TerminalModel boundary.

Generated prost structs are wire DTOs. Public application code consumes validated zterm domain projections, not raw prost values.

### 5.2 Framing

`FrameDecoder` is an incremental pure state machine:

1. read at most 10 varint bytes;
2. reject malformed/overflow length;
3. reject length greater than 8 MiB before reserving the frame body;
4. collect exactly that body;
5. prost-decode `WireFrame`;
6. convert numeric kind through the one kind registry; unknown values fail;
7. for control kinds reject `payload.len() > 1 MiB` before decoding the concrete payload;
8. decode and validate the concrete message once.

The frame body itself is bounded by 8 MiB. The 1 MiB check prevents nested control strings/lists from allocating beyond their control boundary. Terminal snapshot/delta can use the larger frame bound.

`FrameEncoder` uses the same constants and refuses output that the decoder would reject. This is not a second semantic validator; it prevents local code from emitting an invalid frame.

### 5.3 Versioning

- wire major v1 is shared by local IPC and future product ALPN `zterm/1`.
- major mismatch fails before request dispatch with both versions in the diagnostic.
- v1 evolves through optional field additions; unknown protobuf fields are ignored by generated decoders.
- unknown numeric kind is rejected rather than guessed or silently dispatched.
- capabilities negotiate optional service behavior; Agent capability can never become a normal terminal prerequisite.

## 6. Effective user, paths and permissions

### 6.1 Account source

`EffectiveAccount::current()` uses effective UID + account database and returns UID, GID, home and login shell. Environment variables are only candidates for runtime directory discovery and must pass owner/mode/path validation.

`UserPaths` derives:

```text
state_root       ~/.zterm
config           ~/.zterm/config.toml
identity         ~/.zterm/identity.key
database         ~/.zterm/state.sqlite3
install_metadata ~/.zterm/install.json
logs             ~/.zterm/logs
daemon_log       ~/.zterm/logs/daemon.log
lifecycle_lock   ~/.zterm/lifecycle.lock
daemon_lock      ~/.zterm/daemon.lock
runtime_dir      platform candidate or /tmp/zterm-<uid>
socket           <runtime_dir>/daemon.sock
```

Product constructors always derive paths from `EffectiveAccount`. Tests call internal/public library functions with an explicit `UserPaths` built under `tempfile::TempDir`; the installed CLI exposes no path override flag or `ZTERM_HOME` environment escape.

### 6.2 Managed-node checks

- state root/runtime/log directories: directory, current UID owner, no symlink, mode exactly 0700 after creation; existing wider modes are reported, not silently accepted.
- identity/config/database/lock/log files: regular file, current UID owner, no symlink, mode no wider than 0600.
- SQLite uses `SQLITE_OPEN_NOFOLLOW`; ordinary files use no-follow/create-new where the standard platform API exposes it.
- same UID is the product trust boundary. The checks defend against other OS users and accidental path substitution, not a malicious process already running as the same account.

### 6.3 Atomic file write

`atomic_write(path, writer)` creates a unique sibling with create-new and 0600, invokes the caller writer, syncs the file, renames it over the target, then syncs the parent directory. A writer error removes only that sibling and leaves the existing target unchanged.

Identity creation uses the same mechanism but refuses to replace an existing final path. The lifecycle lock prevents concurrent setup; a crash before rename leaves no committed identity, while a crash after rename leaves the complete 32-byte key.

## 7. Configuration and identity bootstrap

### 7.1 Configuration

Minimal v1 TOML:

```toml
schema_version = 1
device_name = "work-mac"

[infrastructure]
profile = "official-n0"
```

Optional self-hosted-only:

```toml
schema_version = 1
device_name = "work-mac"

[infrastructure]
profile = "self-hosted"
relay_url = "https://relay.example.com"
```

The enum shape makes mixed Relay maps unrepresentable. `official-n0` installs only the pinned Iroh production constants/default map. `self-hosted` requires one valid HTTPS RelayUrl and later constructs `RelayConfig::new(url, None)`; it replaces the Relay map and does not imply QAD, while retaining the already approved official production DNS/Pkarr lookup services. No config field enables staging or arbitrary environment selection.

Device names are trimmed, 1–128 UTF-8 bytes, and contain no control character. TOML syntax errors come from `toml`; zterm adds only schema/profile/name diagnostics.

`install.json` is a reserved installer-owned path. Its absence is valid for a development/unmanaged binary in M3; setup neither fabricates it nor treats it as device identity.

### 7.2 Bootstrap state matrix

Setup holds `lifecycle.lock` and follows:

| Existing committed files | Result |
| --- | --- |
| none | create key → DB metadata → config |
| key only | retain key; create DB/config |
| key + DB, no config | validate metadata; create config using existing metadata/name |
| key + config, no DB | retain both; create matching DB metadata |
| all three | validate and return same EndpointId |
| DB and/or config but key missing | hard error; never generate a replacement identity over existing state |
| invalid key or DB/key mismatch | hard error; never rotate or rewrite |

If a daemon is already live, repeated setup asks it to validate current state and returns; it does not open SQLite beside the StoreActor. M3 treats a different requested name/profile on an already complete setup as an explicit conflict, not a silent live reconfiguration. Later device/config commands can own updates.

The key is exactly Iroh `SecretKey::to_bytes()`. EndpointId is derived every time; only its public bytes/string may appear in metadata/status/logs.

## 8. SQLite schema and store owner

### 8.1 Opening

- new database file is precreated 0600, then opened READ_WRITE + NOFOLLOW;
- existing file passes path owner/mode/type checks before open;
- `foreign_keys=ON`, rollback journal, `synchronous=FULL`, bounded busy timeout;
- schema migrations use `TransactionBehavior::Immediate`;
- `PRAGMA user_version` is the single schema version.

No WAL or pool is needed because there is one connection owner and no concurrent read workload.

### 8.2 Schema v1

```sql
CREATE TABLE metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    device_id BLOB NOT NULL CHECK (length(device_id) = 32),
    device_name TEXT NOT NULL,
    created_at_unix INTEGER NOT NULL
);

CREATE TABLE device_auth (
    endpoint_id BLOB PRIMARY KEY CHECK (length(endpoint_id) = 32),
    display_name TEXT NOT NULL,
    status INTEGER NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    paired_at_unix INTEGER NOT NULL,
    revoked_at_unix INTEGER,
    last_seen_at_unix INTEGER
);

CREATE TABLE known_devices (
    endpoint_id BLOB PRIMARY KEY CHECK (length(endpoint_id) = 32),
    local_alias TEXT NOT NULL UNIQUE,
    remote_name TEXT NOT NULL,
    route_cache_version INTEGER,
    route_cache BLOB,
    routes_verified_at_unix INTEGER
);
```

Status is mapped through one daemon enum and SQL CHECK values; callers do not scatter integer meanings. Revocation updates status, increments generation and keeps the row. Route cache remains opaque/versioned until M5 owns its encoding.

There are deliberately no session, PTY, terminal, operation, pair-offer or audit-event tables.

### 8.3 StoreActor

`StateStore` is a synchronous owner used during offline bootstrap/doctor. The running daemon moves it into one dedicated `StoreActor` thread. Tokio request handlers send typed commands and await one-shot results; they never share a `Connection` or wrap it in a global mutex.

If daemon is live, status/doctor use IPC. Offline doctor may open read-only because no StoreActor owns the database.

## 9. Daemon lifecycle and locks

### 9.1 Lock roles

- `lifecycle.lock`: short exclusive lock for setup and launcher singleflight.
- `daemon.lock`: held by one daemon for its full lifetime.

The daemon never acquires `lifecycle.lock`. Launcher code never keeps it while waiting for daemon shutdown. This fixed order prevents launcher↔daemon lock cycles.

### 9.2 ensure_daemon

```text
probe readiness
  if ready -> return
try lifecycle.lock until bounded deadline
  after acquired -> probe again
  if ready -> release and return
  spawn current executable hidden internal daemon mode
  wait for readiness or child failure/deadline
release lock
```

Concurrent losers wait on the same readiness condition and never spawn a second child. No PID file is needed: socket readiness plus lifetime file lock own the observable state.

### 9.3 Internal daemon entry

1. call safe `nix::unistd::setsid()` before runtime/thread initialization;
2. resolve effective account/UserPaths independently;
3. validate state and acquire `daemon.lock`;
4. open/migrate/verify StateStore;
5. inspect existing socket:
   - live connect succeeds: exit as already running;
   - connect fails: require current-UID, non-symlink socket before removing;
   - daemon lock unavailable: never remove socket;
6. bind socket inside 0700 runtime dir and chmod 0600;
7. start StoreActor/DaemonService and accept IPC;
8. on graceful stop, send response, stop accepting, drain bounded work, remove only its socket and release lock.

Unexpected process death releases the OS file lock; the next lock owner performs the stale-socket decision.

### 9.4 Detached child

The launcher sets:

- stdin: `/dev/null`;
- stdout/stderr: append to 0600 `logs/daemon.log`;
- cwd: effective account home;
- args: hidden internal daemon entry only.

The child itself calls `setsid()`; no unsafe `pre_exec`, double fork, launchd/systemd or supervisor. M3 logs only lifecycle/diagnostic events; it performs one simple size check/rotation before a new spawn rather than building a high-volume log subsystem before transport exists.

## 10. Local IPC service

### 10.1 Peer and resource gate

After accept, before decoding:

- Linux: `SO_PEERCRED.uid`;
- macOS: `getpeereid.uid`;
- expected UID: daemon effective UID.

`authorize_peer(expected, observed)` is a shared pure decision function. Wrong UID closes the connection without parsing request bytes. The 0700 directory/0600 socket are the first OS boundary; peer credentials are the authoritative second check.

The listener permits at most 32 active unary connections. Each request has a default 5 s and maximum 30 s relative deadline. Timeouts/cancellation reclaim only that handler.

### 10.2 M3 request kinds

| Request | Side effect | Auto-spawn caller? |
| --- | --- | --- |
| readiness | none | used inside explicit `ensure_daemon` |
| status | none | no |
| validate setup | none | setup already decided to run |
| stop/preflight | lifecycle stop after response | no |
| update preflight | none; schema only for M9 | no public updater yet |

`restart` is a CLI composition: status/preflight → confirmed stop → `ensure_daemon`. It is not a second server command.

Status returns build version, wire major, state schema, EndpointId, device name, profile, daemon start time, and active-session summary. M3 active session count is structurally zero; M4 replaces that projection through the one SessionService.

### 10.3 Cancellation and commit

M3 reads exactly one bounded request frame and enforces the relative deadline
around service dispatch. Readiness, status, setup validation, and update
preflight are read-only. Stop has no durable or session mutation: the server
flushes the response and shuts down that socket before it signals the listener
to exit. If the response cannot be flushed, the stop signal is not sent;
repeating stop (including against an already stopped daemon) is lifecycle
idempotent. The reserved stop `operation_id` is therefore not consumed and M3
does not claim replay behavior.

Starting in M4, cancellation is checked before a `SessionService` mutation
begins. Once create/rename/close/takeover commits, its exact typed result is the
operation result even if the response is lost; a retry with the same
OperationId replays it through the shared `OperationWindow`. No compensating
rollback is invented for an already committed state change.

## 11. CLI contract

M3 command surface:

```text
zterm setup [--name <name>] [--profile official-n0|self-hosted]
            [--relay-url <https-url>]
zterm status [--json]
zterm doctor [--json]
zterm daemon status [--json]
zterm daemon stop [--force]
zterm daemon restart [--force]
zterm logs [--lines <n>]
zterm --help
zterm --version
```

Hidden internal daemon mode is omitted from help.

First setup prompts for missing name/profile only on an interactive terminal; non-interactive setup requires explicit values. Repeated setup returns the current identity without changing it.

No-argument `zterm` prints current milestone help until M4 supplies `connect local --session main`; documentation must not claim a usable terminal early. Inspection commands report configured/stopped/unhealthy state without spawning. `daemon stop` on an already stopped daemon succeeds with an explicit message.

`logs` reads at most a bounded recent tail and prints the log path; it does not start daemon or read secret/SQLite. Human output and JSON are projections of the same typed status, not two independent probes.

## 12. Testing and validation ownership

| Contract | Authoritative evidence |
| --- | --- |
| IDs/principals/resources/operation replay | `zterm-core` unit/state-machine tests |
| frame/kind/version/size compatibility | `zterm-proto` codec tests + generated round trip |
| account/path/mode/symlink/atomic file | `zterm-platform` temp-path tests |
| peer credential retrieval | real same-UID socket on macOS/Linux; one Linux cross-UID CI harness |
| config/key/setup recovery | daemon bootstrap integration with explicit TempDir UserPaths |
| SQL schema/migration/transaction | daemon persistence integration |
| one daemon/stale socket/detach | one multi-process lifecycle harness |
| CLI side-effect table | one CLI integration matrix using injected launcher/client |
| Official N0 profile unchanged | existing `iroh_profile_gate`; no duplicate assertions |
| full portability/policy | existing source/version/fmt/Clippy/test/doc/deny + hosted matrix |

Tests do not mutate real `~/.zterm`. Multi-process tests use their own harness executable arguments to pass TempDir paths; no product environment override is introduced.

The cross-UID Linux harness opens a test-only reachable socket and runs the client as a different existing UID so it crosses the peer-credential gate rather than merely failing at directory mode. It is one security-boundary test, not a general privileged test framework.

No routine network/NAT gate is rerun for this local-only task. Foundation Iroh profile regression is sufficient.

## 13. Error model

Errors retain a stable category plus bounded diagnostic detail:

- `NotSetup`, `AlreadyConfiguredConflict`
- `PathUnsafe`, `PermissionMismatch`, `UnsupportedPlatform`
- `IdentityInvalid`, `IdentityStateMismatch`
- `ConfigSyntax`, `ConfigVersion`, `ConfigProfile`
- `SchemaTooNew`, `MigrationFailed`, `StoreUnavailable`
- `DaemonStopped`, `DaemonAlreadyRunning`, `DaemonStartTimeout`
- `PeerUidMismatch`, `DeadlineExceeded`, `Cancelled`
- `WireMajorMismatch`, `UnknownKind`, `FrameTooLarge`, `ControlPayloadTooLarge`, `MalformedFrame`
- `OperationOutcomeUnknown`, `ServiceNotImplemented`

Secret bytes, full tickets, terminal data and raw SQL are never included. CLI human/JSON and wire error mapping consume the same category owner.

## 14. Migration and recovery

This is schema v1 in a pre-1.0 project. Tests create only task-private state; implementation must not migrate the developer's real home during automated gates.

- atomic config/key writes leave old or new complete state;
- SQLite transaction failure rolls back; no automatic database recreation;
- schema newer than the binary is refused;
- removing/reverting unreleased code is sufficient code rollback; test temp directories can be deleted;
- no rollback drill is run after acceptance merely to prove one exists;
- destructive migration backup/restore belongs to the future migration that actually needs it, not speculative M3 machinery.

## 15. Decisions retained for later milestones

- M4 consumes `TargetSelector::local`, SessionId/AttachmentId/Revision, terminal proto and local connection shape to add the one SessionService.
- M5 consumes DeviceId, pairing proto, device_auth generation/tombstone and official/self-host profile to add Iroh/auth.
- M9 consumes update-preflight/stop response; M3 does not access GitHub or replace binaries.
- M10 performs the real two-network automatic address-discovery test.
- Phase 3 implements Windows daemon/local IPC with current-user ACLs; M2 schemas/types must remain portable but do not emulate Unix sockets on Windows now.
