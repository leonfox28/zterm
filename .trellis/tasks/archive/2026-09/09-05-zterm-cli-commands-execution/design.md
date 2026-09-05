# Design: human-facing CLI, lifecycle and useful daemon logs

Status: implemented and verified by independent check and the full native gate. The user selected all CLI improvements
for this round and then excluded logs -f/--follow. Logging improvements use the
existing recorder and one-shot reader; the narrowed scope preserves the other
authorized work.

## Scope and ownership

Implement PRD R3–R20 through ordered slices in this task. Output, setup, pairing
and confirmation share the CLI parser/dispatch/tests. Update and the new
diagnostic events cross the same daemon and Session lifecycle owners. Keep one
integrated acceptance/commit boundary with
explicit sequential ownership, rather than parallel workers editing those
shared files. A separate task is unnecessary for mere parser spelling aliases;
event coverage has a focused test owner and log reading retains its current one.

- CLI owns arguments, English human output, interaction detection, and reading
  the user's confirmation.
- LocalRuntime owns observation, authenticated update preparation, interruption
  policy, stop/wait, executable activation, and restart/readiness.
- SessionService owns whether shutdown can begin without interrupting live or
  in-progress Session ownership. The local server applies that decision.
- Distribution retains manifest/signature verification and executable trust.
- Existing Session/network/pairing owners emit their committed operational
  events. The daemon retains its log subscriber and the runtime its existing
  one-shot tail reader. No new service or persistent observer is introduced.

The approved PRD supersedes existing spec clauses requiring public --force,
human/JSON output alternatives, and leaving the daemon stopped after update.
Update those clauses during implementation; do not treat their old behavior as
an unresolved permission requirement.

## Public command contract

```text
zterm status
zterm doctor
zterm daemon status
zterm device list
zterm session list [<target>]
zterm setup [--name <name>] [--profile <official-n0|self-hosted>] [--relay-url <url>]
zterm pair accept [--stdin] [--alias <alias>]
zterm update [--version <tag>] [-y|--yes]
zterm daemon stop [-y|--yes]
zterm daemon restart [-y|--yes]
zterm session close <target> <session> [-y|--yes]
zterm device revoke <device> [-y|--yes]
zterm reset --identity [-y|--yes]
zterm uninstall [-y|--yes]
zterm logs [-n|--lines <count>]
```

Remove the five public JSON options and output branches. Remove Serialize/view
types and dependencies only when they no longer have another actual consumer.
There is no substitute machine-output flag. Hidden release self-check JSON and
daemon/core serialization remain intact.

All seven destructive/lifecycle commands share the same input/explicit-yes
policy. `-y` and `--yes` are the same clap field. Public --force disappears from
update/stop/restart/reset/uninstall rather than becoming a hidden alias.

For a live Session impact, use operation-specific English wording, for example:

```text
The following sessions are running:
  main
  build

Updating zterm will end all running sessions. Continue? [y/N]:
```

Flush the prompt before reading. Accept trimmed, case-insensitive y/yes; cancel
on any other answer or EOF. With explicit yes, do not read stdin. Without an
interactive TTY or explicit yes, a nonempty impact returns a cancellation/error
explaining `Run again with -y to continue without prompting.` Empty impact
never reads stdin and does not produce a confirmation prompt.

The empty-impact shortcut above is only for Session interruption during
update/stop/restart. Reset and uninstall combine identity/executable deletion
and active Session impact into one prompt, including with zero Sessions when
there is something to delete. Already-absent identity reset remains a no-op.
Revoke and close retain their exact target/identity preflight before confirming.
Successful confirmation (or -y) authorizes the associated internal interruption
boolean as well as deletion; there is no second force flag or second prompt.

## Readable output and common invocation defaults

Render device lists as aligned English columns with alias/display name first,
observed connection state, outbound connectability and inbound authorization
(including revoked). Preserve each full ID in a following labelled line so
inbound-only records are still actionable. A peer with no active connection is
`Not connected`, not conclusively `Offline`. Do not add active network probes.

Session lists use name, Attached/Detached controller state, viewport and full ID.
Avoid internal revision counters in the default row. Empty lists give an English
description and an actionable command, such as `zterm connect local` for an
empty local Session list. `session list` without target means local; explicit
target parsing and all other positional command layouts remain unchanged.

Status presents device, version, daemon state, infrastructure, high-level
observed network state and Session names/count. The same observation already
contains the detailed fields. Extend doctor to render endpoint bind state,
attempt count, publish/lookup state, direct/relay and connection/stream counters
as English diagnostics alongside its existing local checks. Move information
between these projections rather than dropping diagnostics on JSON removal;
preserve version/full local identity access and all no-autospawn contracts.

Use the existing unicode-width facility for aligned user-visible names. Preserve
exact identities internally; visual layout must not introduce prefix matching.

## Setup and pairing guidance

Default first setup to official-n0 when profile is omitted, regardless of stdin
interactivity. Only device name is prompted in ordinary first interactive setup.
An explicitly selected self-hosted profile still asks for a missing Relay URL
interactively or requires it noninteractively. Existing committed config wins
on repeated setup; the fallback default must never replace a previous
self-hosted configuration. Explain an already-configured conflict in human
terms without pretending setup is a configuration editor.

Rename pair accept's --name to --alias and reject the old spelling. Keep the
existing default alias derivation and collision handling. Success explains
that this machine can connect to the accepted host and prints a connect command
using the actual stored alias. Shell-quote valid aliases containing spaces or
quotes in suggested commands; do not paste arbitrary aliases as bare shell
syntax. State directional trust accurately without changing authorization.

Pair create keeps the ticket as one stdout line. English guidance and requested
TTL (including the actual 600-second default) go to stderr, with receiver-side
`zterm pair accept` instructions. Preserve bounded/no-echo/explicit-stdin secret
handling, and replace the invalid 30s help example with 60s/10m/1h. No need to
decode a ticket in the CLI merely to describe the configured TTL.

## Shared interruption flow

Supply a narrow typed confirmation callback from the CLI into the runtime's
human-facing lifecycle operations. The runtime passes only Session names/counts
and the interruption decision; it does not print prompts or read stdin. Reuse
one path for stop/restart/update, with no generic workflow engine or durable
confirmation state. Exact Rust names may follow nearby conventions.

1. Observe without spawning. A stopped daemon has empty impact.
2. If impact is nonempty and explicit yes was not supplied, invoke the callback.
   Cancellation exits before stop or activation; no lock is held while waiting
   for human input.
3. Send the existing LocalStopRequest with its internal boolean set only when
   interruption was explicitly approved. Removing a CLI flag does not require
   a protobuf rename or wire-major change.
4. If unapproved stop encounters newly admitted work, return a non-stopping
   impact through existing LocalStopResponse fields (`stopping = false`, count
   and names). Map that response to a typed needs-confirmation outcome, not
   successful stop. Refresh/display impact in this same invocation and ask.
5. Successful stop retains bounded Session cleanup and truthful ownership
   release. Restart/update wait for socket and daemon-lock release before
   continuing. Stop never launches a daemon.

Explicit y/yes authorizes this invocation to end all running Sessions, as the
prompt states; it is not a frozen per-Session-ID authorization list. `-y` gives
that same action-level authorization immediately. Recheck failures unrelated
to newly admitted work remain errors, not automatic destructive retries.

### Atomic idle check

Currently service.rs ignores LocalStopRequest.force. Honor that boolean at the
SessionService shutdown admission boundary. Under the existing registry lock,
unapproved shutdown may begin only when no live/provisional/cleanup owner or
Starting reservation exists. Otherwise return the current names/impact without
flipping `accepting` or cancelling owners. Use existing names/ownership maps,
not a parallel cached count. Approved shutdown retains the existing cancellation
and bounded cleanup loop.

This serializes creation admission with the idle decision. A create admitted
first causes an impact response; a stop admitted first closes creation admission
before any new work can start. No new public command or protobuf fields are
needed. Existing direct-client teardown tests that mean unconditional shutdown
must pass true explicitly.

## Update transaction and startup

Preserve one prepared candidate across the complete operation:

```text
validate managed executable / release selection
  -> download and authenticate candidate once
  -> inspect compatible daemon and Session impact
  -> conditional English confirmation or explicit yes
  -> stop and wait for ownership release
  -> acquire lifecycle lock and recheck stopped ownership
  -> activate / verify / write existing metadata / commit
  -> release lifecycle lock
  -> ensure daemon from the activated executable
  -> validate observed readiness against installed manifest
  -> render final result
```

The callback fits into the existing LocalRuntime update owner after preparation;
the CLI never receives a candidate path or install-state authority. Preserve
existing signature, version monotonicity, managed-build, and compatible-current-
daemon checks. Confirmation does not authorize unverified activation.

DaemonLauncher already stores the installed executable path. After replacement,
reusing it launches the new bytes, not the old running updater image. Its
readiness response carries version/wire/schema observations. Validate them
against the authenticated new build identity; do not compare the new version
to the old updater's BuildIdentity. The current task leaves the wire format
unchanged; a future wire-major migration needs its own release contract.

For valid configured state, start the daemon even when it was stopped before
update. Report readiness rather than waiting for Internet/Relay availability.
Do not use daemon restart after activation: an independently auto-started new
daemon should simply satisfy ensure, not be stopped a second time.

For an unconfigured installation, retain binary-only update and return a typed
not-configured startup result, rendered as:

```text
Updated zterm from <old> to <new>.
Run zterm setup to configure and start the daemon.
```

No implicit setup or identity creation occurs. Malformed configured state is
an error rather than a new first-setup opportunity.

When new binary activation committed but daemon startup fails, return a
nonzero, explicit partial-completion error:

```text
Updated zterm to <new>, but the daemon could not start: <reason>.
Run zterm daemon restart to try again.
```

Keep the installed new executable. Existing activation/post-check failure still
rolls back the executable before commit; startup failure after commit does not
invent an additional rollback mechanism. Ended Sessions are never restored.

Emit typed progress from the existing update owner at real preparation,
verification, stop, activation and startup boundaries. The CLI renders concise
English phase text; it never receives a candidate path or adds an unsynchronized
writer to daemon.log. Failures identify their actual phase. No progress timer,
percentage estimator or separate update journal is needed.

## Useful events in the existing daemon log

Reuse tracing and the existing daemon.log writer/subscriber. Configure readable
English records with time, severity and component. Instrument each committed
event at its existing owner, using validated Session names/IDs and safe local
correlation where available:

| Owner | Events |
| --- | --- |
| Daemon lifecycle | ready/stopping/recovered, version and PID at startup, classified startup/cleanup failure |
| Session owner | created, ended with typed exit/close reason, controller attached/detached/taken over |
| Network/broker owner | observed degraded/recovered state, primary connection established/closed, stable failure reason |
| Pairing/authorization owner | operation committed/failed and revoke outcome without bearer material |

Log normal transitions at INFO and actionable failures at WARN/ERROR. Capture
operation/stage and stable error categories, not arbitrary error/source Debug
trees. Do not enable raw dependency debug logging as the default. Emit only
changed network states/committed Session actions; normal detach is not a warning,
and counter/RTT refreshes, input/output frames and replayed mutations do not
create repeated lifecycle events. Avoid logging while holding registry locks.

Respect existing terminal, clipboard, cwd, environment, ticket/key/proof and
remote path/address redaction boundaries. Logging must never fail an otherwise
successful operation or introduce a second Session/connection state owner.
The daemon subscriber records daemon events; CLI update phase output is not
misrepresented as persisted daemon logging during the stopped interval.

Keep daemon.log and daemon.log.1 with the existing startup-time 4 MiB rotation
check. This task does not add a runtime size limiter, extra log files, writer
processes, remote collection, or retention engine. Fill the placeholder logging
guideline with the verified emitter/level/field conventions during implementation.

## One-shot log viewing

Give --lines the -n short spelling and keep LocalRuntime::log_tail as the sole
reader, with its current line/byte limits and no-autospawn semantics. Missing or
empty logs produce a concise English message. Explicit zero lines can remain
an empty selection without implying the underlying log is absent.

The user removed -f/--follow from scope. Do not add a deferred streaming result,
file watcher, polling loop, retained read offset, rotation-follow state machine,
or associated test matrix. More useful recorded events are independent of
continuously watching the file.

## Compatibility and documentation

- Public --json, all public --force flags and pair accept --name become parse
  errors; document -y/--yes and --alias migration. logs -f remains unsupported.
- A previously installed old updater still executes its old UX for the first
  upgrade into this release. The new confirmation/startup behavior applies
  when invoking the new binary; this change cannot retroactively alter an
  already-running old updater. Do not add a new installer channel to hide that.
- Update README, remote CLI and local-daemon docs, install/update docs, and
  owning daemon/distribution/Session specs. Do not rewrite archived task history
  or unrelated `gh --json` calls.
- Include setup defaults, Session list's local default, text tables, consolidated
  destructive confirmation and log event conventions in current docs/specs.
- Tests may continue using typed daemon results. Internal JSON and protobuf
  are implementation contracts, not publicly offered serialization commands.

## Verification owners

- CLI parser/output tests: new public syntax, rejection of removed options,
  text completeness, Unicode/empty tables, direction labels and hidden
  self-check visibility boundary. Verify setup defaults without identity drift
  and pairing hints with a safely quoted alias and one ticket stdout emission.
- CLI lifecycle tests: empty-impact no-read, English names/prompt, y/yes accept,
  cancel/EOF, explicit yes bypass, and noninteractive refusal with live work.
  Extend the existing test-private process/PTY harness for one actual prompt
  and continuation case; do not write to the real account's state.
- SessionService/local IPC tests: an unapproved request cannot close a live
  Session or cancel admitted creation; approved request uses bounded cleanup;
  empty registry can stop. One deterministic admission race owns this invariant.
- Runtime/distribution tests: prepare once before interruption, cancellation
  preserves daemon/binary, configured stopped and running updates finish with
  new-version readiness, no-setup guidance, and post-commit startup failure.
  Reuse existing injected paths, launcher, activation and candidate fixtures;
  do not introduce production trust bypasses to test source builds.
- Logging tests: capture a real committed Session create/end and network
  degrade/recovery with an isolated subscriber. Check useful correlation/reason
  fields, no unchanged-state/normal-detach warning spam, and absence of sentinel
  terminal/ticket/cwd content. Existing log-tail tests own limits and no spawn;
  no follow tests are required.
- `just check` is the final native workspace gate. Existing hosted native jobs
  own Linux evidence and signed Release integration; local macOS must not run
  real Iroh acceptance or claim hosted results.

## Risks and deferred work

Session shutdown admission is the main behavioral risk; preserve registry lock
order, retained cleanup owners, absolute deadlines, and failed-cleanup recovery.
Update restart must not hold lifecycle.lock while ensuring a daemon. Current
setup and signature boundaries remain authoritative. Remote owners gain only
event instrumentation: no transport behavior, Session persistence, terminal
renderer, release publication, broad cleanup refactor or new logging subsystem
belongs in this task.
