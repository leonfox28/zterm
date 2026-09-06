# Remote self-update design

## 1. Outcome and scope

The user permits ending all existing Sessions. After acceptance inside a zterm
PTY, update must complete independently, then start the configured daemon.
The user reconnects manually; ended shells and programs are not restored.

This fixes the update execution boundary described in
`research/current-behavior.md`. No remote-management RPC, Session supervisor,
automatic reconnect, background update check, or always-running service is added.

## 2. One execution owner

Introduce a one-shot internal updater process, launched from the current managed
executable. Use it for all public update invocations, whether in a remote PTY,
local self-attachment, external terminal, or a noninteractive script. Do not
detect remote connections, inspect shell ancestry, or maintain two activation
implementations.

The foreground CLI owns presentation and input. The detached updater owns the
entire candidate preparation and mutation lifecycle. It starts before download
and keeps the existing `PreparedRelease` / `TempDir` alive until completion;
there is no serialized trusted candidate handoff or second download.

Ownership by layer:

| Owner | Responsibility |
| --- | --- |
| `crates/cli/src/lib.rs` / `main.rs` | Public options, hidden internal entry, progress, confirmation, truthful exit status |
| `crates/daemon/src/update.rs` (new) | One-shot updater execution, private control messages, foreground adapter and completion ownership |
| `crates/daemon/src/operations.rs` | Shared preparation/approval/stop/activation/start operation; factor the current body without duplicating it |
| `crates/daemon/src/distribution.rs` | Existing managed-build and signed-candidate verifier and candidate lifetime |
| `crates/platform/src/local_unix.rs` | Process detachment and private parent/child channel primitives |
| Existing lifecycle / user-state owners | Daemon ownership release, atomic activation, rollback, readiness and validated log operations |

`LocalRuntime::update_with_callbacks` remains the frontend adapter. An internal
execution method runs its factored update body in the child; it must not recurse
through the frontend adapter and launch more children.

## 3. Private control boundary

Use one inherited Unix channel between the foreground and child. It is an
ephemeral parent/child connection, not a public socket, daemon endpoint, wire
protocol extension, persistent job queue, or state-path override. The platform
layer owns file-descriptor inheritance and closes unused endpoints on both sides.

The hidden child entry is mutually exclusive with other internal entries and
normal commands. It derives production paths from the effective account. The
foreground validates the managed executable before launching; the child retains
the official-build guard before network or destructive operations. Development
tests use existing explicit test-path/build fixtures, never an environment escape
hatch in the production executable.

Conceptual typed messages, with bounded framing and fixed variants:

- Parent: begin with release selection and `-y` approval; continue into the
  mutation phase; approve or cancel a specific Session impact.
- Child: ready after OS detachment; progress; verified/prepared with target
  version; confirmation request with Session impact; accepted; completed with
  `UpdateResult` or typed domain failure.

The child never reads the controlling terminal. The foreground reuses the
existing y/yes confirmation code. These messages remain local to the updater
module; no changes to terminal protobufs, ALPN, or shared remote DTOs are needed.

## 4. Execution and cancellation contract

1. Validate the official managed executable and version selection. Launch the
   one-shot child with no PTY stdio dependency; it establishes a separate OS
   session before reporting ready. Startup failure leaves the daemon untouched.
2. The child prepares exactly one authenticated release using the existing
   distribution owner. Preparation or verification failure returns normally,
   drops staging, and leaves Sessions and the installed executable intact.
3. The foreground displays verified version, Session impact, and a clear warning
   that this connection may end while update continues. For configured installs,
   include existing `zterm logs` and reconnect/version-check guidance.
4. Obtain interruption approval only when required. Interactive waiting holds
   no lifecycle lock. Cancel/EOF/noninteractive refusal causes child cleanup
   without stopping the daemon. `-y` grants the existing action-wide approval.
5. After preparation and the initial approval decision, the foreground sends
   continue. The child records acceptance and becomes responsible for completion.
   An accepted operation survives subsequent frontend EOF, terminal hangup, or
   lost progress output. Before continue, frontend disappearance cancels at the
   next bounded preparation/control boundary.
6. Keep `stop_with_confirmation`'s registry-admission race behavior. If new work
   appears after an idle preflight and there is no action-wide approval, request
   confirmation again in the same foreground invocation. If the foreground has
   disappeared, cancel without interrupting unapproved work. Continue/handoff
   is never permission to force-stop newly observed work.
7. Stop and await actual daemon ownership release. Reuse lifecycle locking,
   atomic activation, post-check and rollback, metadata commit, and configured
   startup. Release the lifecycle lock before ensuring the new daemon. Match
   readiness against the candidate manifest, not the old child image.
8. Record the actual result and send it to the foreground if still connected.
   Reap the updater while the foreground remains alive. If its parent has gone,
   the one-shot child exits naturally after completion and releases staging.

After acceptance, progress writes must be bounded/nonblocking with respect to
the update operation. A closed or stalled frontend cannot block shutdown,
activation, or startup. Notification failure is not an activation failure.
Loss of the control channel before a required approval still fails closed.

The ordinary external-terminal CLI waits for the final result and preserves
meaningful exit status. It must not return success merely because a child was
spawned. If the channel is lost while the CLI survives, report an unknown final
outcome with version/log guidance instead of assuming success or rollback.

## 5. Diagnostics without new persistent state

For configured installations, the updater appends a small fixed set of typed
stage/outcome records to the existing validated `daemon.log`. This is required
because its frontend is deliberately terminated by update. Reuse existing safe
append/path primitives and the bounded `zterm logs` reader; do not add an update
database, status command, status file, extra log file, watcher, or service.

Do not retain an append descriptor across daemon startup rotation: resolve/open
the current log at each stage, especially the final result after startup, so
`zterm logs` can read completion from the current file. Include version, stage,
an invocation correlation value, and safe error category; exclude terminal
content, environment values, tickets and identity material. Known pre-mutation
log-path failures must be surfaced before stopping. A later logging failure
cannot roll back or misclassify an otherwise committed update.

Before setup there is no affected zterm Session. Preserve the existing no-state-
creation rule and terminal result/setup guidance; do not create an identity,
database or daemon log merely to update an unconfigured installation.

The logging and distribution specs currently describe terminal-only updater
progress and prohibit extra writers. Amend that precise rule to permit this
one-shot update owner's bounded records. This is a necessary change for the
confirmed disconnect workflow, not a general logging redesign.

## 6. Concurrency and recovery

Keep the existing lifecycle lock as the configured activation serialization
owner; add no generic jobs manager or long-held lock during human input.
Under activation ownership, verify that the installed executable still matches
the source identity selected by this invocation, and preserve the existing
restarted-daemon rejection. A competing update that superseded this invocation
must be rejected rather than applying a stale candidate. Verification belongs
in the shared activation boundary, not in both frontend and worker independently.

Existing recovery semantics remain:

- Failed preparation, launch, approval or stop: preserve the installed binary;
  report ended Sessions truthfully if stop partially progressed.
- Activation/post-check/metadata failure: use the existing retained-binary
  rollback. Do not claim that stopped Sessions were restored.
- Committed activation with startup failure: retain the new executable and
  report partial completion with restart/SSH recovery guidance.
- Host crash, OS kill of the updater, or power loss: no promise of durable job
  resumption. Preserve existing filesystem activation guarantees and report
  only evidence available from installed version and logs.

## 7. Compatibility and initial migration

Supported delivery remains macOS arm64 and Linux arm64/x64. This change does not
require remote protocol or state-schema changes, and does not relax current
version compatibility. The helper runs the currently installed updater image,
so an older installed version cannot use this design during its first upgrade.
Document one-time migration via an independent terminal such as SSH, then
subsequent updates can be invoked inside zterm.

## 8. Validation ownership

The decisive regression runs the real update orchestration inside an isolated
daemon-owned PTY and observes target activation and readiness independently
after terminating that PTY. It must fail before the ownership correction.
Preparation must use the existing verifier with fixture fetch/build inputs;
do not replace the update body with a marker-writing fake worker.

Existing distribution fixtures already have a `MapFetcher`, test signing key,
and self-checking candidate archive, but their shell candidate does not implement
daemon startup. Extend the existing harness with a minimal executable candidate
role capable of reporting the fixture identity and entering the existing daemon
test launcher. Inject fixture release inputs at the test boundary; never expose
production URL/key/state overrides. A marker-only detachment test can supplement
this regression, but cannot satisfy A1.

A hosted signed remote acceptance run remains separate evidence from isolated
tests. It requires an installed release containing this fix and a newer signed
compatible candidate. Record this dependency honestly; no automatic release,
server mutation or manufactured signing authority is part of this planning task.
