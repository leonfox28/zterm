# Remote sessions and the public CLI

The M7-M8 command surface is implemented as a thin client of the current
user's daemon. The CLI never reads `identity.key` or SQLite, never owns an Iroh
Endpoint, and never resolves routes itself. Local and remote terminal commands
both enter the daemon's one `SessionService`, so a Session has the same
`SessionId`, PTY, working directory, controller lease, and terminal model
regardless of which authorized device is attached.

This repository is still a development build. The Linux real-Iroh remote
Session gate has been added but its hosted Linux execution and run URL are
pending. Windows keeps shared compile-time and typed unsupported boundaries;
it is not a supported terminal runtime.

## Setup and inspection

The production default and recommended infrastructure profile is Iroh's
official N0 profile. The exact setup syntax is:

```text
zterm setup [--name <name>]
            [--profile <official-n0|self-hosted>]
            [--relay-url <https-url>]
```

The recommended noninteractive invocation is:

```text
zterm setup --name <name>
```

First setup defaults to `official-n0` without a profile prompt. An interactive
terminal asks for a missing device name; noninteractive setup requires `--name`.
Explicit `--profile self-hosted` still requires a Relay URL, prompted when
interactive or supplied with `--relay-url`. Running `zterm setup` with no
overrides after setup preserves the committed identity and configuration and ensures that one daemon
is running. The existing `self-hosted` profile remains an explicit optional
configuration, but it is not the production default and this milestone adds no
public or self-hosted Relay acceptance workflow.

These inspection commands do not start a daemon or create state:

```text
zterm status
zterm doctor
zterm daemon status
zterm logs [-n|--lines <n>]
```

`status` and `daemon status` summarize the device name, version, daemon state,
infrastructure, network state, and running Session names/count. `doctor` shows
the detailed local socket/lock and network observations, effective account,
managed state, and the lack of automatic login startup. Public inspection
output is human-readable text; `--json` is not supported.

`logs` reads a bounded tail once: 100 lines by default, at most 1,000 lines and
1 MiB. `-n` is the short spelling of `--lines`; missing logs produce an English
explanation. There is no follow mode. Existing `daemon.log` records lifecycle,
Session, connection, network and pairing events without terminal content or
secrets. At daemon startup, a log of at least 4 MiB is rotated to `daemon.log.1`;
this is a startup check, not a size cap during a long-running daemon.

Daemon lifecycle commands are:

```text
zterm daemon stop [-y|--yes]
zterm daemon restart [-y|--yes]
```

Stopping an already stopped daemon succeeds. With no running Sessions, stop
and restart proceed directly. Otherwise they list the Session names and ask
for English `[y/N]` confirmation because those Sessions and PTYs will end.
Detached Sessions also need confirmation; connections without Sessions do not.
A newly admitted Session encountered by the stop request is presented in the
same invocation before it can be ended. `restart` explicitly starts a daemon.

Confirmation accepts `y` or `yes`, ignoring case and surrounding whitespace.
Empty input, EOF, or another answer cancels. `-y`/`--yes` skips the prompt;
noninteractive input with live work must supply it. Public `--force` has been
removed. The same input rules apply to close, revoke, reset, uninstall and
update. Reset/uninstall still confirm actual deletion even with no Sessions.

## Pairing and directional trust

Pairing commands are:

```text
zterm pair create [--ttl <duration-with-s|m|h-suffix>]
zterm pair accept [--stdin] [--alias <alias>]
```

`pair create` writes the one-time bearer ticket to standard output once, with
validity and receiving instructions on standard error. The default validity is
10 minutes; valid values range from `60s` to `1h`. Keep the ticket out of command
arguments, environment variables, shell history, logs, and
error reports.

By default, `pair accept` reads one bounded line from an interactive TTY while
echo is disabled. It has no ticket positional argument and no `--ticket` flag.
A non-TTY is rejected without reading unless automation explicitly selects
`--stdin`; that mode reads the bounded standard input to EOF, trims surrounding
whitespace, and rejects input over 16 KiB. `--alias` assigns the exact outbound
alias after acceptance; the old `--name` spelling is rejected. Success reports
the control direction and a `zterm connect` command using the actual stored
alias, quoted for shell use when needed.

Trust is directional. If host A creates a ticket and controller B accepts it:

- A authorizes B to control A (`inbound_status` on A).
- B records A as a device it can connect to (`outbound_known` on B).
- A does not gain permission to control B. Establish that direction with a
  separate ticket from B.

Device management preserves those two directions:

```text
zterm device list
zterm device rename <device> <alias>
zterm device revoke <device> [-y|--yes]
```

The table leads with device names and aliases and retains full device IDs,
both directions of control permission, authorization state, and current
connection observations. An unconnected device is not necessarily unreachable.
An empty list explains how to pair a device. It does not expose route cache
entries, direct addresses, Relay URLs, tickets, terminal content, or working
directories.

`rename` changes only an outbound known-device alias and therefore rejects an
inbound-only record. `revoke` changes only the selected device's authorization
to control this host; its outbound-known record is retained. Revoke disconnects
and detaches that remote principal, but it does not close the host Session or
PTY and does not affect another principal. It uses the shared `[y/N]`
confirmation or explicit `-y`/`--yes`.

An unauthorized remote peer receives only the generic `unauthorized` category.
The wire response does not reveal whether the identity is unknown, revoked, or
using a stale authorization generation.

## Targets and selectors

Every Session command resolves its target inside the local daemon:

- `local` is the reserved target for this daemon and cannot be a device alias.
- A remote target is an exact, case-sensitive outbound alias or a canonical
  full Device ID: exactly 64 lowercase hexadecimal characters.
- Exact aliases are checked before hex-looking short text is rejected. Without
  an exact alias, short/prefix IDs are invalid. A 64-character ID candidate
  must be lowercase; one that is also another device's exact alias is rejected
  as ambiguous.
- Remote Session access requires the outbound-known direction. Inbound
  authorization alone does not make a device connectable.

A Session selector is either an exact, case-sensitive Session name or a
canonical full Session ID of 32 lowercase hexadecimal characters. No prefix
matching is performed.

`local` uses same-UID IPC only. It does not self-dial, create an Iroh
connection, perform DNS/Pkarr lookup, or use a Relay.

## Session commands

The public commands are:

```text
zterm connect <target> [--session <name-or-id>] [--takeover]
              [--escape <ctrl-@..ctrl-_|ctrl-?|none>]
zterm session list [<target>]
zterm session new <target> <name> [--cwd <host-path>]
                   [--escape <ctrl-@..ctrl-_|ctrl-?|none>]
zterm session attach <target> <session> [--takeover]
                      [--escape <ctrl-@..ctrl-_|ctrl-?|none>]
zterm session rename <target> <session> <new-name>
zterm session close <target> <session> [-y|--yes]
```

`connect` defaults to `main`. If `main` does not exist, the daemon atomically
creates and attaches it; otherwise it attaches the existing `main`. A named or
ID-selected `connect` attaches an existing Session. `session new` creates the
named Session and immediately attaches the exact returned `SessionId`; if the
follow-up attach fails, the created Session remains live and its ID is reported.
`--cwd` is interpreted and validated by the selected host, not the controller.

`session list` defaults to `local` when its target is omitted. Its table leads
with names, shows Attached/Detached state, and retains full Session IDs. Empty
results include a creation hint. Other Session commands keep their explicit
target positions.

`session close` ends the exact selected Session and its PTY using shared
`[y/N]` confirmation or explicit `-y`/`--yes`. Rename preserves the Session ID.
Session lists intentionally omit the working directory and terminal content.

There is one controller per Session. An ordinary second attachment never
steals it and returns occupied. `--takeover` first synchronizes a replacement,
then atomically invalidates the previous controller lease. Local attachment has
no priority over remote attachment: remote-to-local and local-to-remote
takeover use the same rule and continue the same Session and PTY.

With no command, behavior is setup-sensitive:

- Before setup, `zterm` prints the fixed `zterm setup` guidance and does not
  create an identity or start a daemon.
- After setup, `zterm` is equivalent to
  `zterm connect local --session main`.
- `zterm --help`, `zterm --version`, parse failures, and all inspection
  commands remain side-effect free.

Commands that need pairing, device, Session, or terminal service validate an
existing setup and then singleflight-start one daemon on demand. They never
perform setup implicitly.

## Attachment, reconnect, and terminal controls

Interactive attach requires both stdin and stdout to be TTYs. Before any
attachment request, the CLI saves the exact terminal attributes, enters raw
mode and a zterm-owned alternate screen, and installs SIGWINCH, SIGINT,
SIGTERM, and SIGHUP handling. Normal detach, typed failure, captured signal,
cancellation, and panic unwind restore terminal modes and display state before
printing a content-free diagnostic. SIGKILL and process abort cannot run this
cleanup and are not claimed as recoverable.

The view moves through preparing, synchronizing, active, and reconnecting
states. A temporary remote transport loss displays:

```text
[zterm: reconnecting]
```

The host Session and PTY continue. After the first successful attach, the
bridge freezes the exact Session ID; reconnect never silently creates another
Session with the same name after host daemon restart or Session termination.
Each reconnect uses a fresh remote attachment identity while the local view
identity remains stable.

While preparing, synchronizing, or reconnecting, the CLI drains and discards
ordinary input instead of queueing it for replay. It retains only the latest
viewport. A full snapshot is written and flushed before its exact revision is
acknowledged; input and resize become effective only in the active state. A
revision gap requests an authoritative full synchronization rather than
guessing missing output. SIGWINCH updates are coalesced.

The default local prefix is `Ctrl+]`:

- `Ctrl+] .` detaches only this view. It does not close or signal the Session.
- `Ctrl+] Ctrl+]` sends one literal `Ctrl+]` byte to the active PTY.
- Any other second byte, or a one-second prefix timeout, sends the pending
  prefix as ordinary input while active.
- `--escape ctrl-@` through `--escape ctrl-_`, or `--escape ctrl-?`, selects
  one other ASCII control byte. `--escape none` disables the prefix.

Because the terminal is raw, ordinary keyboard `Ctrl-C`, `Ctrl-Z`, and similar
control bytes go to the active host PTY. A separately delivered process signal
terminates the local view through the restoration path.

Session state is daemon-lifetime, not disk-persistent. Detach, CLI exit, stream
loss, route change, and device revoke do not end the PTY. Root-shell exit,
explicit `session close`, confirmed daemon stop/restart, identity reset, daemon
crash, and host reboot do. A new daemon starts with no live Session registry.

## Ambiguous operations

Read-only remote requests may retry one byte-identical service request after a
transport ambiguity. Create, rename, close, and takeover reuse the same
daemon-issued operation lease, operation ID, target, and encoded bytes for at
most one remote retry, so a committed response loss replays the exact result.
Operation-lease allocation itself is not retried after a write ambiguity.

If the daemon cannot prove whether a mutation committed, it reports
`operation_outcome_unknown`. The client does not allocate a fresh lease and
silently rerun that logical operation. Inspect current state before deciding on
a new, independent command.

## Identity reset

```text
zterm reset --identity [-y|--yes]
```

Identity reset is destructive but does not uninstall the binary. It reports
the current public identity and running Session names, then asks once to end
the Sessions and delete the identity. Use `-y`/`--yes` to confirm directly.
Existing identity data requires confirmation even with no Sessions; an already
absent state root is a successful no-op.
It then performs a bounded daemon stop, obtains the lifecycle lock, rechecks
the exact identity and daemon state, and removes only the validated managed
state root. No `RevokeSelf` is sent and setup is not run automatically.

After reset, run `zterm setup` to create a different identity and pair again.
The signed installer, explicit updater, and uninstaller reuse this local
lifecycle boundary; see [Install, update, and uninstall](install.md).

## Transport acceptance boundary

The production default remains `OfficialN0`. The Linux-only
`two_daemon_transport` test composes two task-private production transport
owners, but deliberately builds its test Endpoints with `RelayMode::Disabled`,
IPv4 loopback, and a task-only direct route. It proves pair/normal ALPN
isolation, Endpoint/primary reuse, directional authorization, and direct-route
non-persistence. It is not evidence for the remote Session workflow, the
official-N0 Relay, public Internet traversal, an optional self-hosted Relay,
DNS/Pkarr discovery, two physical networks, NAT migration, or the public CLI as
multiple OS processes.

Developer macOS runs may compile this target but must never execute its real
Iroh case. Hosted Linux owns its actual runtime result. A broader remote
Session or pairing workflow target should be added only together with its
hosted job and run evidence, using the same fixture instead of another
daemon-like harness. Windows CI is currently paused; restoring shared
compile/Clippy evidence requires an explicit task and a hosted Windows run.
A macOS cross-compile cannot substitute for it.
