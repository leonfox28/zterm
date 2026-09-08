# Execution plan

Status: detailed startup stages and simultaneous configured file logging are
implemented. Focused presentation, file/rotation and local-wire tests pass;
the final expanded native gate passed with exit 0 on 2026-09-08. Corrected
physical smoke remains a documented follow-up. The user subsequently explicitly
requested creating and merging the PR on 2026-09-08, accepting progression with
that disclosed validation limit.
The user approved implementation and explicitly requested the detailed-stage,
no-time/no-shortcut, simultaneous-file-log refinements. Work remains inline.

## Ordered work

- [x] Load trellis-before-dev and the color, local-daemon/IPC and input contracts.
- [x] Perform the isolated reproduction in research/findings.md. Capture actual
  query output and visible behavior, establish the leakage mechanism and update
  its root-cause classification before choosing a fix. Reconfirm material scope
  changes instead of patching from the screenshot's byte pattern alone.
- [x] Correct the demonstrated emitter/framing boundary and add its focused
  failing-before/passing-after regression. Preserve probe coverage and deadlines.
- [x] Add the small startup presentation state and sole-presenter rendering.
  Wire real initialization/prepare/initial-sync transitions, resize and
  cancellation waiting feedback into the existing waits without elapsed labels.
- [x] Retire startup state coherently on first validated surface/activation;
  preserve semantic history, input epochs, error cleanup and reconnect content.
- [x] Run focused checks, the full native quality gate and disposable local
  acceptance. Controlled local/remote-labelled waits cover slow preparation.
- [ ] Receive corrected Ghostty physical smoke; a real remote connection smoke
  has not been performed. Keep these limits separate from native test evidence.
- [x] Use trellis-check and update executable contracts with trellis-update-spec.
- [x] User authorized the work commit, task bookkeeping, PR creation and merge.
  Required GitHub checks remain the merge gate.

## Detailed-stage follow-up

- [x] Add fixed core stages, optional client observer and bounded journal.
- [x] Observe actual broker dial/reuse/handshake/service-stream boundaries using
  its existing per-peer state, demand owner and notification.
- [x] Forward opt-in same-UID stages before tunnel Opened and remote unary final
  replies; validate stage/correlation/payload/count and preserve decoder leftovers.
- [x] Thread operation/Session events into the sole startup presenter. Remove
  the shortcut hint and preserve first-frame/input/resize/cancellation contracts.
- [x] Append the same typed events and final startup result to configured
  daemon.log, with correlation, rotation reopening and best-effort failures.
- [x] Distinguish early Session end from local cancellation; retain typed failure
  categories and exactly one initial outcome after observer retirement.
- [x] Cover burst-driven rendering, wire fragmentation/coalescing/ordering,
  bounds/retirement, file rotation/no-content/no-setup and retained results.
- [x] Extend the real local PTY harness to assert the full persisted stage order.
- [x] Add real-Iroh fresh/reuse integration coverage under existing platform gates.
- [x] Complete the expanded `just check` and record its final result.

## Validation

- Existing baseline: `cargo +1.98.0 test -p zterm-cli host_colors --lib` — 7 pass.
- During edits: `cargo +1.98.0 test -p zterm-cli --lib`.
- Add a controlled pending-prepare test that checks actual emitted startup cells,
  target/stage without elapsed time, resize and clean first-frame replacement. A test
  of only the state enum or copied label strings is insufficient.
- Exercise cancellation/error restoration, stateful prepare completion and no
  startup-input replay with existing PTY fixtures, extending only affected cases.
- Probe tests must exercise the actual output emitter and relevant consumer;
  slot-substring coverage alone cannot validate terminal compatibility.
- Required final native gate: `just check`.
- Physical smoke: exact Ghostty build, standalone queries, disposable local
  target and authorized disposable remote target, slow prepare, failure/cancel
  and successful first frame. Do not turn headless evidence into a visual pass.

## Implementation and current evidence

- `ansi_presenter` emits all palette indices as separately terminated OSCs in
  the existing single round/write/flush. The new framing regression fails when
  only the original 32-index emitter is restored, and passes with the correction.
- `startup.rs` uses the sole presenter for initialization, target, preparation
  and stateful cancellation feedback. Initial snapshot/ACK waiting
  uses a local synchronization status; Active retires that status.
- The startup physical baseline is separate from authoritative semantic/history
  fallback. Shared disposable terminal replay validates actual displayed cells.
- Pending local and remote-labelled preparation tests verify readable stages,
  cancellation feedback, dropped startup typing and exact retained Session ID;
  output failure also preserves the submitted result. These controlled waits do
  not claim a real remote network or physical terminal smoke.
- The real disposable local-daemon connection harness passes. Its old status
  assertion searched a contiguous raw-byte substring, which is invalid after
  incremental startup-to-session painting. It now replays the actual outer
  output and checks the final status cells before detach.
- Focused startup tests: 3 passed. Full CLI library in the first native gate:
  92 passed, 3 isolated helper tests ignored. Workspace Clippy with all targets
  and features passed. The complete `just check` rerun passed with exit 0 on
  2026-09-08 after correcting the local-daemon acceptance assertion. This includes
  source/release policy, workspace tests, documentation, dependency policy and
  relay static checks. Cross-host and final release evidence remain CI-owned.
- User follow-up: remove all visible waiting times. Removed startup start-time
  storage and its periodic repaint deadline, and replaced elapsed sync state with
  a boolean. Updated exact replay assertions for preparation, synchronization and
  cancellation; the protocol/cancellation deadlines retain their existing owners.
  All 3 focused startup tests and the complete `just check` passed after this
  refinement (exit 0, 2026-09-08).
- User-run original Ghostty reproduction plus macOS OOM diagnostics establishes
  the query failure. A second `--run --candidate` probe was requested; its output
  has not arrived. No installed executable or existing user daemon was replaced.

## Final detailed-stage evidence (2026-09-08)

- Final `just check`: exit 0, including the early-Session-end refinement.
  Output: `/tmp/zterm-detailed-connection-final-check.log`. Workspace Clippy
  (all targets/features), tests, docs, source/release/secret/dependency policies
  and relay static verification all passed under the existing native gate.
- CLI library: 93 passed, 3 intentionally isolated helper tests ignored. The
  actual outer-PTY `daemon_autospawn` harness passed its exact 15-stage file-log
  assertion for the connected CLI PID, and its actual input/resize/detach checks.
- Shared client: 76 passed. Daemon library: 173 passed, 1 isolated helper ignored.
  Progress tests cover bounded burst history, clone retirement, typed outcomes,
  byte-by-byte and coalesced tunnel/unary prefixes, final reply ordering and
  post-final/unsolicited/unknown/correlation/count violations.
- File tests passed for configured logging, per-invocation correlation, rotation
  reopening, typed failure, no raw target content, no setup/path creation and
  unsafe-log refusal. Failed progress writes retain the submitted operation.
- Added broker fresh/reuse real-Iroh coverage; it compiled and was intentionally
  ignored on macOS under the existing firewall-prompt policy. It is runtime
  coverage for Linux CI, not a claimed successful live remote connection here.
- No corrected Ghostty candidate result has arrived; no live-user daemon,
  Session or installed executable was changed. Physical Ghostty/remote smoke
  remains separate from native automated evidence. The user explicitly authorized
  committing, creating the PR and merging after this limit was disclosed.

## Risk and review points

- `crates/cli/src/terminal_ui.rs`: startup/ACK wait and lifecycle contracts;
  arbitrary restructuring can regress stateful cancellation or sole stdin reader.
- `crates/cli/src/terminal_ui/ansi_presenter.rs`: one stdout owner and committed
  semantic/physical baselines; startup text must not become history fallback.
- `crates/cli/src/terminal_ui/host_colors.rs`: change only if actual probe/reply
  evidence requires it; preserve timeout/fence and late-reply semantics.
- Composition/status changes must use real geometry and current transport state.
- Computer-use access to Ghostty was denied in planning. Use a permitted isolated
  non-UI diagnostic or request a manual smoke at the final validation boundary;
  do not bypass that denial with another UI automation mechanism.
- If the leakage mechanism cannot yet be reproduced, keep the investigation open
  and report that limit. A progress-screen improvement alone does not complete R1/R3.

## Approved work commit and PR

`fix(cli): show and log real connection stages and bound color queries`

One cohesive local commit contains the startup presentation/probe correction,
fixed observation DTOs and same-UID transport, actual connection stage producers,
configured logfile sink, regression/acceptance tests and synchronized specs/task
artifacts. The user requested publication through a PR and merge after required
checks. This does not replace a user-installed executable or existing daemon.

Files currently in scope:

- `.trellis/spec/backend/core-wire-domain.md`
- `.trellis/spec/backend/local-daemon-ipc.md`
- `.trellis/spec/backend/logging-guidelines.md`
- `.trellis/spec/backend/shared-client.md`
- `.trellis/spec/backend/terminal-colors.md`
- `.trellis/tasks/09-08-desktop-connection-progress/check.jsonl`
- `.trellis/tasks/09-08-desktop-connection-progress/design.md`
- `.trellis/tasks/09-08-desktop-connection-progress/implement.jsonl`
- `.trellis/tasks/09-08-desktop-connection-progress/implement.md`
- `.trellis/tasks/09-08-desktop-connection-progress/prd.md`
- `.trellis/tasks/09-08-desktop-connection-progress/research/findings.md`
- `.trellis/tasks/09-08-desktop-connection-progress/research/outer-terminal-results.json`
- `.trellis/tasks/09-08-desktop-connection-progress/research/probe-byte-analysis.json`
- `.trellis/tasks/09-08-desktop-connection-progress/research/probe_outer_terminal.py`
- `.trellis/tasks/09-08-desktop-connection-progress/research/retrospective.md`
- `.trellis/tasks/09-08-desktop-connection-progress/task.json`
- `crates/cli/src/terminal_ui.rs`
- `crates/cli/src/terminal_ui/ansi_presenter.rs`
- `crates/cli/src/terminal_ui/composition.rs`
- `crates/cli/src/terminal_ui/host_colors.rs`
- `crates/cli/src/terminal_ui/session.rs`
- `crates/cli/src/terminal_ui/session_tests.rs`
- `crates/cli/src/terminal_ui/startup.rs`
- `crates/cli/src/terminal_ui/startup_tests.rs`
- `crates/cli/tests/daemon_autospawn.rs`
- `crates/cli/tests/support/outer_terminal.rs`
- `crates/client/src/lib.rs`
- `crates/client/src/pair_framing.rs`
- `crates/client/src/progress.rs`
- `crates/client/src/protocol.rs`
- `crates/client/src/remote_unary.rs`
- `crates/client/src/session.rs`
- `crates/core/src/connection_progress.rs`
- `crates/core/src/lib.rs`
- `crates/daemon/src/client/ipc.rs`
- `crates/daemon/src/client/session.rs`
- `crates/daemon/src/client/transport.rs`
- `crates/daemon/src/connection_broker.rs`
- `crates/daemon/src/connection_progress.rs`
- `crates/daemon/src/lib.rs`
- `crates/daemon/src/local_ipc.rs`
- `crates/daemon/src/operations.rs`
- `crates/daemon/src/remote_session.rs`
- `crates/daemon/src/remote_tunnel.rs`
- `crates/daemon/src/service.rs`
- `crates/daemon/tests/connection_broker.rs`
- `crates/proto/src/connection_progress.rs`
- `crates/proto/src/lib.rs`
- `proto/zterm/v2/local.proto`
- `proto/zterm/v2/wire.proto`
