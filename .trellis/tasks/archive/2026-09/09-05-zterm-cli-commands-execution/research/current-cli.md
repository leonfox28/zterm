# Current Zterm CLI inventory

Inspected on 2026-09-05 at commit `6588a74`, workspace version `0.1.18`.
This records current repository behavior, not a proposed command redesign or
a claim about a separately installed executable.

## Evidence

- `crates/cli/src/lib.rs:28`: parser settings, hidden flags, help/version.
- `crates/cli/src/lib.rs:133`: 12 public top-level commands.
- `crates/cli/src/lib.rs:177`: public argument and subcommand definitions.
- `crates/cli/src/lib.rs:639`: dispatch and bare invocation.
- `crates/cli/src/lib.rs:700`: pairing and directional device management.
- `crates/cli/src/lib.rs:770`: terminal connection and Session dispatch.
- `crates/cli/src/lib.rs:870`: reset, update, and uninstall handlers.
- `crates/cli/src/lib.rs:1312`: first setup prompts and committed defaults.
- `crates/cli/src/main.rs:8`: hidden pre-runtime entry dispatch.
- `crates/cli/src/terminal_ui.rs:146`: stdin/stdout TTY requirement.
- `crates/daemon/src/operations.rs:564`: validate setup, then ensure daemon.
- `crates/daemon/src/operations.rs:618`: device list also ensures daemon.
- `crates/daemon/src/operations.rs:690`: Session list resolves configured target.
- `crates/daemon/src/operations.rs:1084`: local doctor checks.
- `crates/core/src/pairing.rs:31`: default/min/max ticket TTL.
- `crates/cli/tests/command_side_effects.rs:18`: existing command-surface and
  no-autospawn assertions (read, not executed during this inventory).
- `docs/remote-cli.md`, `docs/install.md`, `README.md`: user-facing contracts.

## Public operations

The 12 top-level entries expand to 21 concrete operations. In the table,
`<target>` means `local` or an outbound-known remote device's exact alias/full
Device ID. `<session>` means an exact Session name/full Session ID.

| Command | Current behavior |
| --- | --- |
| `zterm setup [--name <name>] [--profile <official-n0\|self-hosted>] [--relay-url <https-url>]` | Configure the device identity/infrastructure and ensure its daemon. First interactive setup prompts for omitted values and defaults the profile to official-n0. First noninteractive setup needs name and profile; self-hosted also needs its relay URL. Repeating setup without overrides preserves committed identity/configuration. |
| `zterm status [--json]` | Observe setup, daemon, active Session count, and available network state without starting the daemon. |
| `zterm doctor [--json]` | Inspect setup, automatic-start limitation, observed network state, effective account home/login shell, managed paths, and IPC. It does not initiate Internet probes or start the daemon. |
| `zterm pair create [--ttl <duration>]` | Print a one-time pairing ticket. Default TTL is 600 seconds; accepted nonzero durations are 60–3600 seconds with s/m/h suffixes. Explicit zero is rejected. |
| `zterm pair accept [--stdin] [--name <alias>]` | Read a ticket from a no-echo interactive TTY, or bounded stdin to EOF with explicit --stdin. Record the ticket issuer as an outbound-known device, optionally with the supplied alias. Tickets are not accepted as positional arguments or a --ticket option. |
| `zterm device list [--json]` | List device identities, aliases, directional trust, and observed connection/attachment state. Starts the configured daemon on demand. |
| `zterm device rename <device> <alias>` | Rename only the outbound alias. An inbound-only record cannot be renamed. |
| `zterm device revoke <device> [--yes]` | Revoke that device's authorization to control this host and detach its remote principal. Preserve outbound-known state and host Sessions/PTYS. |
| `zterm connect <target> [--session <session>] [--takeover] [--escape <prefix>]` | Attach main by default, creating it atomically if absent. Explicit --session main has the same create-if-absent behavior. Other Session names/IDs must already exist. |
| `zterm session list <target> [--json]` | List live Sessions on the selected target. Starts the configured local daemon on demand. |
| `zterm session new <target> <name> [--cwd <host-path>] [--escape <prefix>]` | Create a named Session and immediately attach its returned ID. The selected host interprets cwd. If a follow-up attach fails after creation, the Session remains live and its ID is reported. |
| `zterm session attach <target> <session> [--takeover] [--escape <prefix>]` | Attach an existing Session only, including when the requested name is main. |
| `zterm session rename <target> <session> <new-name>` | Rename a Session while preserving its ID. |
| `zterm session close <target> <session> [--yes]` | End the exact Session and its PTY after confirmation. |
| `zterm daemon status [--json]` | Dispatch to exactly the same status handler as top-level status. |
| `zterm daemon stop [--force]` | Stop the daemon; already stopped succeeds. Refuse active Sessions unless force is explicit. |
| `zterm daemon restart [--force]` | Stop then explicitly start the configured daemon. Refuse ending active Sessions without force. |
| `zterm logs [--lines <n>]` | Print a bounded recent log tail without starting a daemon. Default 100 lines, capped at 1000 lines and 1 MiB; no follow flag. |
| `zterm reset --identity [--yes] [--force]` | Stop the daemon and remove validated managed identity/configuration/pairing state. Preserve the executable; do not automatically rerun setup or notify peers with RevokeSelf. |
| `zterm update [--version <tag>] [--force]` | Download and verify latest stable or an exact newer stable/prerelease Release, then activate it. Reject same-version installs/downgrades. Require force if active Sessions would end. Leave daemon stopped on success. Official managed Release builds only. |
| `zterm uninstall [--yes] [--force]` | Remove validated managed state and the exact running executable. Require force if active Sessions would end. Official managed Release builds only. |

## Other entry points

- Bare `zterm`: before setup, print setup guidance without creating state;
  after setup, behave as `zterm connect local --session main`.
- `zterm -h` / `--help`: help; command-specific `--help` is also available.
- `zterm -V` / `--version`: build version.
- The dedicated `help` subcommand is disabled. Public definitions contain no
  command aliases such as `ls` or `rm`.
- `--internal-daemon`: enter the detached daemon runtime.
- `--internal-release-self-check`: emit side-effect-free build identity JSON
  for installer checks.
- `--internal-release-verify <MANIFEST> <SIGNATURE>`: verify an exact signed
  manifest against the candidate build identity.
- `--internal-release-install <DESTINATION>`: atomically install the current
  verified candidate without overwriting an existing destination.

The last four flags are hidden internal maintenance/installer entries.

## Execution semantics relevant to redesign

- Setup and daemon restart explicitly ensure a daemon. Pair/device/Session
  service operations and interactive connections start one on demand after
  setup validation. They do not initialize identity implicitly.
- Status, doctor, daemon status, logs, daemon stop, help/version, and parse
  failures do not start a daemon. Read-only device and Session lists differ.
- The local target uses same-UID IPC. Remote Session traffic goes through the
  local daemon; the CLI does not own an Iroh endpoint or resolve remote routes.
- A creates a ticket and B accepts: B may control A, not automatically the
  reverse. Device revoke removes inbound control permission, not both trust
  directions or a generic device record.
- Targets and Session selectors use exact case-sensitive names/full IDs,
  without ID-prefix matching. Device IDs are 64 lowercase hex characters;
  Session IDs are 32 lowercase hex characters.
- Connect/new/attach require both stdin and stdout to be TTYs. No public exec,
  run, detached-create, daemon-start, or shell-completion command is defined.
- The default local prefix is Ctrl+]. Ctrl+] followed by `.` detaches the view
  while preserving the Session/PTYS. Repeating Ctrl+] sends one literal prefix.
  `--escape` accepts ctrl-@ through ctrl-_, ctrl-?, or none.
- Each Session has one controller. A second controller is rejected unless it
  explicitly uses --takeover; takeover replaces the controller after sync.
- Detach, CLI exit, transport loss, and device revoke preserve Sessions.
  Root-shell exit, explicit close, daemon termination/crash/restart, identity
  reset, or host reboot end them. Sessions are daemon-lifetime, not restored
  from disk on a new daemon.
- --yes skips the explicit confirmation required by revoke, close, reset,
  and uninstall. --force separately permits ending active Sessions during
  stop/restart/reset/update/uninstall.

## Discussion candidates, not approved requirements

- Default entry behavior and how initialization leads into daily use.
- Overlap between connect and session attach; immediate attachment in new.
- Duplicate status entry points and mixed daemon-start effects of queries.
- Discoverability of directional pairing, aliases, and exact target selection.
- Whether existing scripts need compatibility aliases or a migration period.

## Verification scope

Cross-checked parser definitions, dispatch, daemon facade, existing tests, and
repository docs. No product commands that change real device state were run;
no build or test run was necessary for this source inventory. Product code is
unchanged. Concrete redesign and implementation remain to be specified.
