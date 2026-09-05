# v0.1.16 terminal synchronization and presentation defects

Baseline: `13600e67952e2c448dcf0f1241609e2366c03b58` (v0.1.16).
Owner: main agent only. No production source edits during planning.

These probes and source anchors describe the uncorrected baseline. The planning
injection/observation scripts intentionally depend on that source shape; they are
not acceptance runners for the corrected code. Completed fix evidence is recorded
in [../validation.md](../validation.md) and [retrospective.md](retrospective.md).

## Reproduction

```sh
python3 .trellis/tasks/09-05-fix-terminal-sync-scroll/research/run_presentation_probe.py
```

The runner builds the actual CLI test dependencies, copies CLI source under ignored
`target/terminal-sync-scroll/cli-source-copy`, and injects task-local observation
cases. It does not edit `crates/`. The copied UI implementation is unchanged; only
extra probe tests are inserted. Two tests fail as expected on this baseline.
Raw output: `target/terminal-sync-scroll/presentation-probe.log`.

### R1: queued ordinary delta is mistaken for an activation barrier

The probe uses the real `SessionService`, a temporary PTY fixture, and the current
CLI delta application and ACK decision. It obtains an ordinary emitted delta,
retains it as an in-flight viewer event, resizes the real attachment, and processes
the queued delta while the UI reports `Synchronizing`. The real attachment is
still `Active` because no replacement snapshot has been produced yet.

Observed: `ordinary_delta=0..1 resize=23x80 inferred_ack=true` followed by
`DomainErrorKind::NotSynchronized`. `session.rs:3484` identifies the exact rejection:
`attachment is not awaiting a snapshot`.

This first probe is a deterministic application-neutral boundary reproduction.
The subsequent real CLI/Herdr evidence below establishes the actual startup
cascade, including the stale-ACK replacement snapshot branch that this first
probe did not exercise. Local/remote timing can change exposure; it does not
justify two terminal synchronization contracts.

Causal path:

1. `crates/cli/src/terminal_ui/session.rs:89-126` and `:531-577` submit physical or
   mode-driven resize and change the UI state before all queued output is consumed.
2. The driver retains ordered pending events in `crates/daemon/src/client/view.rs:699-780`;
   a resize command cannot retroactively reclassify an already-emitted delta.
3. `crates/cli/src/terminal_ui.rs:1416` answers the ACK question using only the UI
   transport state. Capturing that state at event entry fixes only the first delta
   that triggers resize; it does not fix a following queued delta.
4. A valid remote-resume delta is identified in the correlated attach response at
   `crates/daemon/src/client/session.rs:579-594`, then reduced to the same event as
   ordinary stream deltas at `:1105-1116`. The UI has lost the fact it needs.
5. The actual server `resize` at `crates/daemon/src/session.rs:3650-3668` changes the
   model but does not itself move an attachment to Awaiting. `next_update` at
   `:3523-3557` establishes Awaiting when producing a replacement snapshot.
6. The incorrectly emitted ACK is rejected by the correct strict server contract
   at `crates/daemon/src/session.rs:3470-3505`.

Classification: **client-to-UI boundary defect**. The existing SessionClient knows
which update is a resume barrier, but its event representation discards that fact.
Restore that distinction at its existing owner; no protocol relaxation, transport
fork, additional interpreter, application detection, or new framework is needed.

#### Local/remote parity audit after the user's architecture question

The terminal work contract is shared, as the user expects. Local IPC enters
`SessionWireServer` at `crates/daemon/src/local_ipc.rs:397`; the authenticated
remote handler owns the same server at `crates/daemon/src/session_wire.rs:354-400`.
Both routes process ACK and resize in `process_attachment_frame` at `:1163` and
`:1239`, ending at the same SessionAttachment/SessionService operations.

The route adapters differ in framing and scheduling: Direct reads Session frames;
Tunnel unwraps opaque envelopes before handing the same frames to SessionClient
(`crates/daemon/src/client/transport.rs:16-34`, `:195-251`). The remote pump also
crosses a viewer-daemon task and network stream (`remote_tunnel.rs:74-113`). This
does not imply Session messages are reordered. Ordered bytes do not impose one
global order between incoming output, frontend resize requests and server update
production. The shared UI incorrectly lets that relative order decide an ACK.

Remote does not tolerate this error: a decoded ServiceErrorResponse is returned
as an error by `client/session.rs:1039-1055`, and NotSynchronized is absent from
the reconnectable kinds at `:1445-1455`. The driver forwards that error as a
terminal outcome (`client/view.rs:951-955`). A received NotSynchronized response
therefore cannot be explained away as automatic remote recovery.

The existing
`client::session::tests::direct_and_tunnel_adapters_share_one_session_trace_and_command_interpreter`
test was rerun on the unchanged baseline with Rust 1.98.0, all features, locked
offline dependencies: 1 passed, 202 filtered. It compares equal decoded delta,
clipboard and sync-required events and equal target-visible ACK, resize, input
and sync commands. It verifies adapter parity for one sequential trace, not the
problematic UI/server interleaving or actual remote network behavior.

The actual CLI/Herdr observations below now establish the same failure on both
real routes. The user's earlier successful remote run was not recorded; its
precise event order cannot be reconstructed retrospectively. No claim that a
particular network latency caused that past success is made. The reproduced
failure does not require a local-only branch, a version mismatch or an error
suppression/recovery branch on remote.

### R1: actual CLI/Herdr startup and paired-device observations

Tools are task-local `cli_runtime_probe.rs`, `run_cli_herdr_probe.py`,
`run_paired_herdr_probe.py` and `instrument_cli_probe.py`. They link the baseline
product CLI/daemon libraries. The local fixture supplies a task-private /bin/sh
PTY spawner and user-state paths while exercising production CLI execution,
`run_terminal`, local IPC, SessionService, driver and presenter. The paired probe
uses the existing authorized `dev` route and creates/closes only a uniquely named
`zterm-causal-*` Session. It exercises actual Iroh transport to the Linux target;
this is distinct from the Linux-owned simulated two-daemon CI fixture.

The user confirmed the remote CLI and running daemon are 0.1.16 and Herdr is
0.8.2. Local read-only checks independently confirmed the same versions. Local
is macOS arm64; the paired probe observed Linux 6.12.101+deb13-amd64 x86_64.
Installed local Herdr 0.8.2 SHA-256 is
`3e0f0c2d5edc41f592963ef90f5d872db801cc7dbd0e01731023897ee428904a`.
No product source file was edited. Observation builds change only ignored source
copies, adding timestamped revision/event metadata; they do perturb scheduling,
so unchanged-library reproductions are recorded separately.

| Route / mode | Geometry | Result | Ignored evidence under `target/terminal-sync-scroll/` |
| --- | --- | --- | --- |
| Local, Herdr `--no-session`, unchanged libraries | 24x80 | Three startup successes | `herdr-cli-outcomes.json`, `herdr-cli-{0,1,2}.ansi` |
| Local, Herdr `--no-session`, unchanged libraries | 50x180 | First two startup successes; third run hit a probe cleanup-write/exit race and is excluded | `herdr-50x180/` |
| Local, default persistent Herdr, unchanged libraries | 50x180 | Three startup failures with the exact reported error | `herdr-50x180-persistent/` |
| Local, default persistent Herdr, observation copies | 50x180 | Three same failures with complete client/server traces | `herdr-50x180-persistent-observed/` |
| Local, default persistent Herdr, observation copies | 40x140 | Two same failures | `herdr-40x140-persistent-observed/` |
| Local, Herdr `--no-session`, observation copies | 40x140 | Startup and exit succeed, Herdr content verified | `herdr-40x140-monolithic-observed/` |
| Actual dev route, isolated persistent Herdr, observation copies | 50x180 | Same failure and delta/ACK cascade | `zterm-causal-c227ca1599/` |
| Actual dev route, isolated persistent Herdr, unchanged libraries | 40x140 | Herdr visible, exact error, then client exits | `zterm-causal-acb89307f6/` |
| Actual dev route, isolated persistent Herdr, observation copies | 40x140 | Same failure and delta/ACK cascade | `zterm-causal-4b5a686ccd/` |

One earlier paired run (`zterm-causal-327ce9cd25/`) is **invalid as startup
evidence**: a fixed two-second pre-input delay was shorter than establishment,
so the input fence discarded the script. It showed neither the script execution
marker nor Herdr content. The runner now requires a round-trip shell readiness
marker and records script-start and Herdr-visible evidence. The unchanged-library
40x140 failure recorded `startup_exit: null` because EOF/error output preceded
the process-reap observation; its captured `PROBE_CLI_ERROR=not_synchronized...`
is decisive and must not be counted as a successful startup.

The 40x140 monolithic positive-control trace shows the distinguishing order:
snapshot 28 → ACK 28 → resize → snapshot 29 → ACK 29 → ordinary delta 29→36
consumed in Active with `ack=false`. In the failing persistent-mode runs, an
ordinary delta instead crosses the resize synchronization interval and is
consumed with `ack=true`. Thus both safe and failing orders have been observed;
the correction must make the ordinary delta's ACK semantics independent of that
relative order. No added observer or transport delay is proposed as a fix.

The previous black-box used `--no-session`, which Herdr's own CLI describes as
monolithic execution. Normal `herdr` uses its persistent client/server path.
The mode changes the produced startup sequence and exposed another coverage gap;
it is a regression input, never a product-side application/mode detection rule.
The paired fixture starts the isolated Herdr server explicitly, waits for its
socket and then attaches the ordinary Herdr client, with cleanup ownership and
a bounded server watchdog. Local default-mode reproduction also covers Herdr's
automatic server start.

Concrete local trace (`herdr-50x180-persistent-observed/herdr-cli-0-trace.log`):

1. Server sends snapshot revision 24 during Herdr startup. UI installs it and
   acknowledges 24. Its Alternate-screen layout requires width 180 instead of
   Main's 179, so the pending resize is flushed by the Active transition
   (`terminal_ui/session.rs:707-753`). UI remains Synchronizing for that resize.
2. An ordinary delta 24→41 crosses from the server while the resize is processed.
   The driver queues it as an ordinary delta, but UI consumes it in Synchronizing
   and logs `ack=true`, then sends ACK 41.
3. The server has already produced resize snapshot 42 and is Awaiting(42).
   ACK 41 takes the stale-revision recovery branch at `session.rs:3486-3499` and
   queues a second snapshot 42. This is not the immediate Active rejection seen
   in the first neutral probe; it is the same invalid ACK taking another branch.
4. UI applies/ACKs the original 42, activating the attachment. It then applies/ACKs
   the replacement 42. The final ACK reaches `awaiting=None` and fails with the
   user's exact `attachment is not awaiting a snapshot` error.

The remote 40x140 trace is equivalent: snapshot 25 → ACK 25 → resize → ordinary
delta 25→30 consumed with `ack=true` → ACK 30 → snapshot 31 twice → ACK 31 twice
→ the same error. The target is the unmodified running remote daemon; observer
metadata here covers only the local frontend, and no target-state logging is
claimed for that run.

**Classification remains architecture / client-to-UI boundary defect.** The
authoritative update origin was erased, leaving UI transport state to decide
whether an ordinary update can acknowledge a barrier. Both the immediate-reject
and replacement-cascade cases arise from that missing invariant. D1 is sufficient
for this demonstrated chain because the first incorrect ACK must never exist;
it must also preserve legitimate full-snapshot and correlated resume-delta ACKs.
Regression acceptance must include the observed snapshot→Active→deferred-resize
path as well as the previously planned delta-triggered/physical resize paths.

All isolated local daemons were stopped. Paired test Sessions were closed through
their exact unique names; a final session-list check showed only the pre-existing
`main`, with the same ID, revision 772, 39x139 viewport and no controller as before
the final paired runs. No existing Session was attached or taken over.

Re-run examples (baseline source, Rust 1.98.0):

```sh
python3 .trellis/tasks/09-05-fix-terminal-sync-scroll/research/run_cli_herdr_probe.py --runs 3 --rows 50 --columns 180 --persistent
python3 .trellis/tasks/09-05-fix-terminal-sync-scroll/research/run_cli_herdr_probe.py --runs 3 --rows 50 --columns 180 --persistent --observed
python3 .trellis/tasks/09-05-fix-terminal-sync-scroll/research/run_paired_herdr_probe.py --target dev --rows 40 --columns 140
```

#### Follow-up: match the user's existing-Herdr workflow

The user clarified that successful remote use means entering a zterm shell and
then typing `herdr` to attach to an already running Herdr server. It does **not**
mean attaching zterm directly to a Session already displaying Herdr. The earlier
new-server paired reproduction establishes common failure, but does not by itself
explain that original success. Keep those two claims separate.

Further controls:

- `zterm-causal-d28e16beeb`: zterm reattachment to an already running isolated
  Herdr also failed under the observation build. This is a sibling case, not the
  user's clarified workflow and not evidence explaining their success.
- `herdr_client_prime.py` starts a client in its own PTY, verifies an actual pane
  input/output marker, detaches with Herdr's prefix-q and verifies the isolated
  server remains running. The zterm shell then launches a **new** Herdr client
  against that primed server, matching the clarified startup sequence.
- `zterm-causal-871567a59f` (dev) and `zterm-causal-4814e598a0` (local) use the
  user's **installed release executable** `/Users/huyuanzhe/.local/bin/zterm`,
  the existing product daemons, 40x140 outer PTYs, and isolated primed Herdr state.
  Both display Herdr and then fail with the exact error. This removes debug-build
  scheduling as a necessary cause and shows that server warmness alone does not
  explain the original local/remote contrast. These are real release-binary
  reproductions, unlike the earlier unchanged-source debug-library probes.
- Read-only local `herdr status server --json` and remote inspection through the
  unique Session `zterm-causal-779f778918` confirm the user's existing Herdr servers
  are both 0.8.2, protocol 20, compatible and do not need restart. Remote shell
  `type -a herdr` identifies only `/home/localadmin/.local/bin/herdr`, with no
  alias. Remote TERM=xterm-256color, COLORTERM=truecolor, TERM_PROGRAM and
  HERDR_SESSION unset. Inspection does not attach to or stop that existing server.

No claim that warm/cold startup, zterm reattachment, binary optimization, a Herdr
server-version mismatch or a shell alias explains the user's original successful
run is supported by these controls.

The user subsequently retested the original remote workflow and reported:
“现在远程连接也不正常了，明明刚才是好的”. The original environment is therefore
not a stable remote-success control. This observation is consistent with the
already recorded schedule-dependent shared ACK defect, and removes the premise
that Local fails while Remote is intrinsically correct. It does not retrospectively
measure the event order of the earlier successful run or establish a particular
RTT, batching threshold or transport path as its cause. Acceptance must prove
correctness under both safe and previously failing orders, not depend on making
the failure less likely.

After the user's retest, git still showed only the untracked planning task;
the local daemon remained v0.1.16 with its original `started_at_unix=1788582357`.
All eight earlier named paired/installed-CLI test Sessions had exact-name close
receipts. Remote main's ID remained unchanged; its revision/viewport changed from
772 / 39x139 to 796 / 39x140 after the user's own new Herdr attempt. Test Sessions
never attached to or took over main. Temporary session creation can affect shared
connection activity and scheduling, so the evidence does not assert that probe
traffic had zero transient timing impact. No product fix, release, daemon restart
or persistent change to the user's Herdr configuration was performed.

The follow-up read-only inspection `zterm-causal-eb22f9fceb` confirmed remote
daemon v0.1.16 still had `started_at_unix=1788582355`, exactly matching the user's
pre-investigation status. Its unique inspection Session was also closed, and
remote main remained at revision 796 / 39x140. Thus neither daemon was restarted
during this investigation. This inspection's path display was relay with varying
RTT, reinforcing that no one latency value should be treated as a correctness
condition; it is not a measurement of the earlier Herdr startup's path.

### R2: logical Live resumes while old history content remains committed

The probe renders labeled live rows, scrolls and presents a history frame at offset
2, returns to offset 0, observes a replacement snapshot, completes Active, and
checks actual committed child cells before any extra input. It repeats for Local
and Remote metadata, with and without new terminal output.

All four cases report:

```text
live_before_click=false kept_history=true active_painted=false live_after_click=true
```

The final boolean uses the same presentation call reached by pointer-driven
rendering; no extra model output is introduced. This proves stale presentation,
rather than lost terminal rows or an arithmetic subtraction of two rows. The
retained offset determines how many newest lines appear missing.

Causal path:

1. `ViewportController::start_resume` (`terminal_ui.rs:2220-2233`) retains the last
   painted history frame and enters `ResumePending`.
2. `observe_snapshot` (`:2242-2249`) updates live metrics and a snapshot flag, but
   deliberately retains the input fence until Active.
3. The snapshot handler (`terminal_ui/session.rs:475-525`) uses ordinary
   `install_snapshot_stdout` (`terminal_ui.rs:2989-3010`).
4. `ComposedFrame::compose_inner` (`terminal_ui/composition.rs:192-231`) considers
   only `viewport.is_live()` for ordinary composition. ResumePending has no history
   slice after cache invalidation, so it copies the old presentation's child rows.
   Chrome can advance to offset zero while content remains the history frame.
5. `finish_resume` (`terminal_ui.rs:2287-2300`) makes the viewport logically Live.
   `present_transport_transition_with_writer` (`:3151-3173`) then unconditionally
   suppresses rendering whenever `resumed_from_snapshot` is true.
6. A subsequent pointer presentation composes the live surface and repairs the
   visible content, matching the user's observation.

Classification: **local presentation contract violation**. The existing single
compositor/presenter architecture already requires replacement content and chrome
in one committed frame. Its snapshot path must explicitly use the live candidate
while preserving the separate input fence. Active must suppress only an actually
unchanged presentation, not assume that resume already painted every component.

## Why earlier checks did not distinguish these failures

- `delta_acknowledgement_uses_event_entry_state_before_mode_resize` at
  `terminal_ui.rs:3772-3797` tests only the triggering delta and repeats a state
  predicate. It never processes an ordinary queued delta after resize.
- The real CLI `daemon_autospawn` scroll scenario (`:774-805`) checks scroll-up
  progress and an unchanged child revision, then detaches. It does not scroll back
  to live and inspect the final visible child rows.
- `tests/foundation/terminal-blackbox.sh:73-74` runs the daemon
  `terminal_blackbox` adapter. That adapter uses `TerminalDriver` directly
  (`crates/daemon/tests/terminal_blackbox.rs:13-16`), so a green Herdr result there
  does not establish CLI snapshot ACK or host presentation behavior.

## Remaining acceptance evidence

Before declaring the fixes complete, turn these observations into maintained tests
at the owning boundaries, exercise interleaved ordinary updates and valid remote
resume barriers through the actual typed driver/UI path, and extend the existing
outer-PTY CLI harness to inspect return-to-live row content before a click. Run
real Herdr through the CLI path, not only the driver adapter. Existing background
history, resize, input/paste fencing, strict server ACK and write/flush-failure
contracts must remain covered.
