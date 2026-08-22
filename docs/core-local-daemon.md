# Core and local daemon milestone

This milestone implements Phase One M2–M3. It freezes shared domain and wire
contracts and delivers a normal user's per-account local daemon. It does not
yet provide a terminal session or remote connection.

## Supported commands

| Command | Starts a daemon | Purpose |
| --- | --- | --- |
| `zterm setup --name <name> --profile official-n0` | yes | Idempotently create/validate state and wait for readiness |
| `zterm setup --name <name> --profile self-hosted --relay-url <https-url>` | yes | Select one explicit self-hosted Relay profile |
| `zterm status [--json]` | no | Report running, configured/stopped, or not configured |
| `zterm doctor [--json]` | no | Check account home/shell, committed state, socket/lock state, and lifecycle limits |
| `zterm daemon status [--json]` | no | Same typed state projection as `status` |
| `zterm daemon stop [--force]` | no | Flush a graceful stop response; stopped is success |
| `zterm daemon restart [--force]` | yes | Stop, wait, then explicitly start one daemon |
| `zterm logs [--lines <n>]` | no | Read at most 1,000 recent lines and 1 MiB |

Running `zterm` with no command prints milestone help. The hidden daemon entry
accepts no state-path argument and is omitted from help.

First noninteractive setup requires both `--name` and `--profile`; self-hosted
also requires `--relay-url`. An interactive terminal prompts for missing
values and recommends `official-n0`. Repeating setup without overrides returns
the existing public identity and starts the daemon only when it is stopped.

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
until an explicit setup/restart or future connection command calls the same
on-demand launcher.

Local unary IPC uses one Unix connection per request and the shared bounded
protobuf framing. The daemon authorizes the peer UID before decoding bytes:
Linux uses `SO_PEERCRED`; macOS uses `getpeereid`. It permits at most 32 active
unary connections, defaults requests to five seconds, caps relative deadlines
at 30 seconds, caps frames at 8 MiB, and caps control payloads at 1 MiB. The
client half-closes its write side after one frame so the server can reject all
trailing request bytes before dispatch. A stop
acknowledgement is flushed before the listener exits and removes its socket.
M3 stop has no durable/session mutation and does not use the reserved operation
replay window; failed acknowledgement delivery leaves the daemon running, while
stopping an already stopped daemon is a CLI-level success. Replay integration
begins with M4 stateful session create/rename/close/takeover operations.

Readiness and status are local-only. They do not create or bind an Iroh
endpoint and do not access DNS, Pkarr, Relay, or the Internet.

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

The profiles cannot be mixed. Neither profile is contacted by the M3 local
daemon; M5 consumes this validated choice when network transport is added.

## Deliberate exclusions

- No `SessionRegistry`, session/tab creation, PTY attach, or `connect local`.
- No Iroh endpoint bind, NAT traversal, pairing, remote authorization, or revoke RPC.
- No GUI, Android, Windows daemon, or iOS client in this milestone.
- No boot/login autostart and no persistence of live work across daemon restart.
- No Agent-specific state recognition or notification behavior before 2.0.

M4 must route local attachment through the same future `SessionService` that a
remote adapter will use. It must not add self-pairing, self-Iroh dial, or a
second local session registry.

## Focused verification

```bash
cargo test -p zterm-daemon --test persistence
cargo test -p zterm-daemon --test setup_idempotency
cargo test -p zterm-daemon --test local_ipc
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
