# Core and local daemon milestone

This document describes Phase One M2–M3: shared domain/wire contracts and a
normal user's per-account local daemon. Later milestones now build the public
local/remote CLI on this same owner. See
[Persistent session engine and local attachment](persistent-sessions.md) for
Session lifetime and [Remote sessions and the public CLI](remote-cli.md) for
the complete current command surface.

## Local lifecycle and inspection commands

| Command | Starts a daemon | Purpose |
| --- | --- | --- |
| `zterm setup --name <name>` | yes | Idempotently create/validate state and wait for readiness |
| `zterm setup --name <name> --profile self-hosted --relay-url <https-url>` | yes | Select one explicit self-hosted Relay profile |
| `zterm status` | no | Report running, configured/stopped, or not configured |
| `zterm doctor` | no | Check account home/shell, committed state, socket/lock state, and lifecycle limits |
| `zterm daemon status` | no | Same typed state projection as `status` |
| `zterm daemon stop [-y|--yes]` | no | Flush a graceful stop response; stopped is success |
| `zterm daemon restart [-y|--yes]` | yes | Stop, wait, then explicitly start one daemon |
| `zterm logs [-n|--lines <n>]` | no | Read at most 1,000 recent lines and 1 MiB |

After setup, running `zterm` with no command attaches local `main`; before setup
it prints fixed setup guidance without creating an identity. `zterm --help`
always prints help. The hidden daemon entry accepts no state-path argument and
is omitted from help.

First setup defaults to `official-n0`. Noninteractive setup requires `--name`;
interactive setup prompts for a missing name without asking for a profile.
Explicit self-hosted setup also needs `--relay-url`, prompted interactively if
omitted. Repeating setup without overrides returns
the existing public identity and starts the daemon only when it is stopped.

Stop/restart proceed directly with no live Sessions. Otherwise the CLI lists
the names and asks for English `[y/N]` confirmation; `-y`/`--yes` skips input.
Detached Sessions count as live work. The daemon atomically checks Session
admission before accepting an unapproved idle stop, so a concurrent creation
cannot be ended based on an earlier empty observation.

`logs` reads recent records once and never starts the daemon. Key lifecycle,
Session, network, connection and pairing events use the existing daemon log.
The existing startup check rotates a log of at least 4 MiB to `daemon.log.1`;
there is no continuous reader or runtime size cap.

## Per-user state

Persistent paths are derived from the effective UID's account database, not
`$HOME` or `$SHELL`:

```text
<account-home>/.zterm/config.toml
<account-home>/.zterm/identity.key
<account-home>/.zterm/state.sqlite3
<account-home>/.zterm/install.json       (reserved for the installer)
<account-home>/.zterm/logs/daemon.log
```

Managed directories are mode `0700`; files and the Unix socket are no wider
than `0600`. Managed symlinks, wrong owners, unexpected node types, and wider
permissions are rejected. Atomic config/key writes use a same-directory
create-new temporary file, file sync, rename, and directory sync.

`identity.key` is exactly 32 raw Iroh `SecretKey` bytes. Setup never replaces
an existing identity. If config or the database exists without the key, setup
fails instead of generating a new identity over committed state. Removing the
whole installation state during uninstall therefore invalidates old pairing
identity; reinstall creates a new endpoint identity. No private key bytes are
placed in logs, SQLite metadata, status, or JSON.

SQLite uses the bundled library, one live `StoreActor`, rollback journal,
foreign keys, `synchronous=FULL`, `SQLITE_OPEN_NOFOLLOW`, and transactional
`PRAGMA user_version` migration. Schema v1 contains only public identity
metadata, device authorization generations/revocation tombstones, and a
versioned route cache. It does not persist PTYs, terminal bytes, sessions,
operation replay windows, or pairing offers.

## Daemon and local IPC

There is one daemon per OS user and one installed executable. `lifecycle.lock`
serializes setup/launch briefly; `daemon.lock` is held for the process lifetime.
The daemon does not acquire the lifecycle lock, and there is no PID-file kill
fallback.

The launcher redirects stdin to null, appends stdout/stderr to the managed log,
uses the account home as cwd, and the child calls safe `setsid()` before Tokio
runtime initialization. There is no systemd, launchd, cron, login item,
supervisor, or automatic update. After a crash or reboot, no daemon starts
until an explicit setup/restart or a configured pair/device/connect/Session
command calls the same on-demand launcher.

Local IPC uses the shared bounded protobuf framing. The daemon authorizes the
peer UID before decoding bytes:
Linux uses `SO_PEERCRED`; macOS uses `getpeereid`. It permits at most 32 active
connections, defaults requests to five seconds, caps relative deadlines
at 30 seconds, caps frames at 8 MiB, and caps control payloads at 1 MiB. The
M2–M3 lifecycle calls and M4 session mutations remain strict unary requests:
the client half-closes its write side after one frame so the server can reject
all trailing request bytes before dispatch. A `TerminalAttachRequest` instead
selects one long-lived duplex stream while retaining the same decoder and peer
gate. Session work runs outside the current-thread runtime with one absolute
deadline, so a full actor mailbox or blocked PTY cannot stall socket progress.
Readiness, status, and session list allocate no replay state. A logical client
lazily obtains a daemon-incarnation/monotonic-ordinal operation lease before its
first mutation. Only an ambiguous transport failure gets one byte-identical
retry; a complete typed error is definitive and never silently moves the same
mutation to another lease.

A stop acknowledgement is produced only after bounded concurrent session
cleanup succeeds, then flushed before the listener exits and removes its
socket. Cleanup deadline or failed acknowledgement delivery leaves the daemon
and listener running for status/retry, while stopping an already stopped daemon
is a CLI-level success. M4's create/rename/close/takeover mutations use
per-operation exact-result singleflight; a completed request whose response was
lost replays on a new socket without repeating its side effect.
Recoverable listener accept errors stay inside the serve loop. On fatal server
exit the daemon removes its socket only after all live and provisional session
owners are released. Failed cleanup retains the process, daemon lock, store,
service, and children; it compare-rebinds the exact device/inode/change-time
socket token it
published and restores status/stop retry. A replaced same-UID socket path is
never unlinked by that recovery loop.

Readiness and status are local-only observations. They neither wait for nor
initiate Endpoint bind, DNS/Pkarr, Relay, or Internet work. A running production
daemon separately owns its Iroh Endpoint and publishes truthful degraded
network state without making local IPC unavailable.

`doctor` uses those same local owners without creating files or starting the
daemon. It validates the account home and login shell, managed path ownership
and modes, identity/config/database consistency, and whether socket and lock
state agree with the observed daemon state. On Linux it also reports that
`systemd-logind` host policy may end an on-demand daemon after logout; it does
not enable linger or install an autostart unit.

## Infrastructure profiles

The default config is Iroh's pinned official N0 production profile:

```toml
schema_version = 1
device_name = "work-mac"

[infrastructure]
profile = "official-n0"
```

The optional alternative replaces that Relay map with exactly one HTTPS Relay
and disables QUIC address discovery for that entry:

```toml
schema_version = 1
device_name = "work-mac"

[infrastructure]
profile = "self-hosted"
relay_url = "https://relay.example.com"
```

The profiles cannot be mixed. The current production network owner consumes
this validated choice; `OfficialN0` remains the default and recommended
profile. The optional self-hosted profile is not an additional M7-M8 acceptance
workflow.

## Deliberate exclusions

- This M2–M3 document does not own installer, updater, uninstaller, or release
  artifact contracts. See [Install, update, and uninstall](install.md) and
  [Release operations](releasing.md) for their current owners.
- No M10 two-physical-network, NAT/path-migration, or new public Relay gate.
- No GUI, Android runtime, Windows daemon/ConPTY, or iOS client in this
  milestone.
- No boot/login autostart and no persistence of live work across daemon crash,
  restart, upgrade, or host reboot.
- No Agent-specific state recognition, observer mode, multi-writer controller,
  or disk transcript.

Local attachment uses the same `SessionService` as the authenticated remote
adapter. The `local` target does not self-pair, self-dial Iroh, perform address
discovery, or create a second Session registry.

## Focused verification

```bash
cargo test -p zterm-daemon --test persistence
cargo test -p zterm-daemon --test setup_idempotency
cargo test -p zterm-daemon --test local_ipc
cargo test -p zterm-daemon --test local_session_ipc
cargo test -p zterm-daemon --test session_concurrency
cargo test -p zterm-daemon --test terminal_recovery
cargo test -p zterm-daemon --test single_instance
cargo test -p zterm-daemon --test detached_lifecycle
cargo test -p zterm-cli --test setup_permissions
cargo test -p zterm-cli --test daemon_autospawn
cargo test -p zterm-cli --test command_side_effects
sh tests/core-local-daemon/cross-uid.sh
```

The cross-UID command skips without noninteractive privilege on local machines.
Under CI, missing noninteractive privilege is a failure; the Ubuntu x64 job
owns the real `nobody` socket-reachability rejection.
