# Design: explicit synchronization updates and atomic return-to-live presentation

Status: implementation approved (2026-09-05); actual local and paired remote Herdr failure chains recorded.
Installed-release existing-server controls and the user's original-workflow retest
also fail remotely. No stable local/remote correctness distinction remains;
earlier successful-run timing is not retrospectively claimed. Shared-contract
design is approved for implementation by the main session.
Baseline and causal evidence: [research/causal-evidence.md](research/causal-evidence.md).

## Boundaries and root causes

The R1 probe and actual CLI/Herdr traces demonstrate loss of synchronization
meaning at the existing SessionClient -> typed view boundary. The same invalid
delta ACK produces an immediate rejection or a stale-ACK/replacement/duplicate-ACK
cascade, depending on target progress. Both real routes reproduced the reported
exit; successful prior remote use is not evidence of a different correct contract.
R2 is a violation of the existing compositor/presenter contract. Keep one
SessionClient, one authoritative SessionService and one DesktopPresenter for both
routes; revise the proposed scope if the remaining causal investigation finds a
different or additional missing invariant.

## D0 / R6: establish cause before committing to the correction

Completed evidence is recorded in `research/causal-evidence.md`: unchanged-library
local and actual paired-device failures, metadata-only observation-copy traces,
startup-mode controls, same product versions and scoped fixture cleanup. The
unrecorded historical successful remote run cannot be reconstructed and is not
attributed to a specific latency. The following rules remain acceptance constraints:

- Observe an uncorrected real CLI/Herdr startup in isolated user-state paths and
  retain its outcome. Instrument only isolated source copies when necessary;
  record event metadata, not unrelated terminal contents or credentials.
- Compare versions, application/input and viewport before attributing a route
  difference to transport. Trace target updates and sync state, client event
  enqueue/consume, resize and ACK with revisions and attachment/epoch correlation.
- Use a controlled Direct/Tunnel interleaving to distinguish a route-specific
  implementation branch from a common scheduling-dependent invariant violation.
  A sequential adapter parity test or a remote route label is insufficient.
- Identify the exact invalid transition in the real failure and tie it to the
  application-neutral regression. Preserve unexplained behavior as an evidence
  gap; do not assert that network latency explains it without observing it.
- Only then settle the owning contract and correction. No route/application
  symptom branches, ignored errors, retry masks or timing-based repair. If the
  actual trace contradicts D1, reconverge the plan before product edits.

## D1 / R1: acknowledge an explicit barrier, not a UI transport state

- Preserve the distinction between an ordinary delta and the correlated initial
  delta of a resumed attachment. Use a typed resume-update case (for example
  `ResumeDelta`) in the existing local/client and terminal-view event contract.
- Only SessionClient's correlated reconnect establishment can produce that resume
  case. Ordinary session-stream deltas cannot gain ACK semantics because a resize,
  sync request, queued state event or presentation transition changed UI state.
- Share semantic delta validation/application code. After successful presentation,
  acknowledge full snapshots and explicit resume barriers exactly once. Ordinary
  deltas update only the applied revision used for recovery.
- Remove `delta_acknowledges_existing_sync` and its state-only reasoning. Keep
  `Synchronizing` for input gating/coalesced resize; it does not grant ACK authority.
- Cover the real snapshot -> Active -> deferred resize transition, not only a
  delta that changes modes. A queued ordinary delta must not trigger the target's
  stale-ACK replacement path or any resulting duplicate snapshot confirmation.
- Maintain server strict Awaiting + exact revision validation, no input replay,
  reconnect recovery and takeover rules. The wire format stays unchanged: the
  reconnect response's existing request correlation already carries the needed
  distinction to SessionClient.

## D2 / R2: compose the replacement live candidate while input remains fenced

- The snapshot install path chooses its content source explicitly. A replacement
  for ResumePending uses the new live surface and offset-zero metrics in the same
  composed transaction; pinned History during a background sync remains pinned.
- Reuse `ComposedFrame` and its live candidate projection instead of adding another
  renderer or fixing rows after composition. The last complete history frame
  remains visible until a valid replacement can be committed.
- Treat successful output/flush as the presentation boundary. The new snapshot's
  surface, selected layout and live-content presentation must agree before ACK;
  failed output cannot advance a presented baseline or forward retained input.
- Keep the input/paste fence until the existing activation event. Live content may
  be presented before the input owner becomes Active; these are separate facts.
- Replace unconditional `resumed_from_snapshot` paint suppression with an actual
  desired-frame comparison at DesktopPresenter. Preserve the no-op optimization
  when content, chrome, cursor and modes are already correct; allow any needed
  cursor or mode transition. Do not depend on a later mouse click or terminal output.

## Expected implementation files

- `crates/daemon/src/client/session.rs`: retain correlated resume-update meaning.
- `crates/daemon/src/client/view.rs`: forward the typed update through the existing
  driver and verify ACK/state ordering, including a queued ordinary delta.
- `crates/cli/src/terminal_ui/session.rs`: consume explicit barriers and pass the
  correct snapshot presentation intent without weakening input gates.
- `crates/cli/src/terminal_ui.rs` and `terminal_ui/composition.rs`: one snapshot
  composition/commit path, actual Active-frame difference checks, maintained
  presentation regressions and removal of obsolete state-only ACK logic.
- `crates/cli/src/terminal_ui/session_tests.rs`: exercise actual UI/driver/target
  interleavings and explicit resume application with isolated state and PTYs.
- `crates/cli/tests/daemon_autospawn.rs`: extend the real outer-PTY acceptance path;
  reuse existing fixture helpers and add only the deterministic controls required
  to reproduce the identified schedules. Existing daemon tests consuming the
  resume variant may need mechanical expectation updates.
- Corresponding `local-daemon-ipc.md` and relevant `session-service.md` spec entries:
  preserve the owner, barrier, presentation and evidence contracts.

## Acceptance and limits

Test application-neutral mechanisms first; retain default persistent Herdr as a
real CLI smoke case and `--no-session` as a separate control. Cover both route
metadata and remote resume; a local-host unit test is not real network evidence.
Repeat the existing paired dev acceptance in uniquely named disposable Sessions,
with scoped Herdr cleanup. The Linux-owned simulated two-daemon CI test remains a
separate platform gate; actual paired-device evidence does not replace it.

No new crate, wire version, client framework, server-ACK tolerance, application
recognition, render delay or unconditional repaint loop. No version bump or release
in this initial scope. The changes have no persisted-state migration and can be
reverted together if the surrounding ACK or presentation contracts regress.
