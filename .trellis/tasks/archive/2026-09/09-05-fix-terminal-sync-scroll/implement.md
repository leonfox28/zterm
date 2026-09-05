# Implementation plan

Execution owner: main agent only. Explicit user instruction forbids all subagents,
including Trellis roles. Context manifests are reading lists, not dispatch requests.

## 0. Planning evidence

- [x] User approved creating this task and entering planning.
- [x] Inspect the previous snapshot fix, current source, specs and existing gates.
- [x] Reproduce stale return-to-live content for both routes, with/without new output.
- [x] Reproduce queued-delta ACK rejection with real SessionService and current UI logic.
- [x] Persist requirements, root-cause classification, design and validation boundaries.
- [x] Complete D0: connect an actual CLI/Herdr baseline failure to the neutral
  regression; compare route traces and controlled interleavings to explain the
  differing observations. Record environmental differences and remaining gaps.
- [x] Reassess whether D1 restores the complete invariant identified by D0;
  revise classification/scope if the evidence identifies additional root causes.
- [x] Incorporate the user's exact existing-Herdr remote retest: the original
  remote environment now fails too. Installed-release primed-server controls also
  fail on both routes. Preserve the unrecorded successful-run timing as a limit,
  not an unsupported claim that a particular network latency caused success.
- [x] User approved D1/D2 after the causal investigation (2026-09-05); start implementation.

## 1. Make the missing ACK distinction testable

- [x] Start a dedicated fix branch from the recorded baseline/current main.
- [x] Reload `trellis-before-dev` and current task evidence in the main session.
- [x] Add a deterministic driver/UI regression for an ordinary delta queued before
  resize but consumed after it; assert exact outgoing command kinds and server result.
  Exercise the controlled interleaving through both Direct and Tunnel adapters;
  sequential trace parity alone does not validate scheduling independence.
  Include the observed snapshot -> Active -> deferred resize path and both server
  states: Active immediate rejection and Awaiting stale-ACK/replacement cascade.
- [x] Cover a legitimate correlated remote resume delta to prevent overcorrecting
  by dropping every delta ACK. Preserve full-snapshot and takeover acceptance.
- [x] Implement D1 in SessionClient -> view -> CLI, share application logic, and
  remove the state-only ACK helper and tautological helper test.

Focused validation:

```sh
cargo +1.98.0 test -p zterm-daemon --lib --all-features
cargo +1.98.0 test -p zterm-daemon --test controller_lease --test local_session_ipc --test attachment_resync --all-features
cargo +1.98.0 test -p zterm-cli --lib --all-features
```

## 2. Restore the actual live presentation

- [x] Add maintained tests for committed child rows through History -> ResumePending
  -> SyncRequired -> Snapshot -> Active, before any extra click/input/output.
- [x] Include both route labels, quiescent/new-output cases, cursor and chrome,
  pinned background sync, and the existing failed write/flush boundary.
- [x] Implement D2 using the existing candidate/compositor/presenter path. Preserve
  delayed input/paste release and avoid unnecessary identical frame transactions.
- [x] Extend the existing real CLI outer-PTY scroll case to return to live and
  inspect its final screen, not only revision progress or a presentation count.

## 3. End-to-end validation and retrospective

- [x] Exercise real CLI startup/screen switches with default persistent Herdr on
  local and the actual paired dev route; retain --no-session as a separate control.
  The old driver black-box alone is insufficient; use the outer-PTY CLI path and
  uniquely owned test Sessions and Herdr state, preserving all existing Sessions.
- [x] Verify the shared route contract and valid remote reconnect using existing
  socket/transport fixtures; keep actual paired-network evidence separate from
  the Linux-owned simulated two-daemon test gate.
- [x] Run `trellis-break-loop` retrospective: why the earlier tests passed and which
  owning regression now distinguishes each failure. Main agent performs it inline.
- [x] Update executable specs to match the final event and presentation contracts.
- [x] Run one final `just check`, explicit included-module rustfmt check and
  `git diff --check`; only rerun broader checks for actual changes or failures.
- [x] Record exact evidence and remaining platform limits.
- [x] Phase 3.4: user approved [commit-plan.md](commit-plan.md) and the subsequent release flow on 2026-09-05; commit the reviewed work now.

Additional commands as applicable:

```sh
cargo +1.98.0 test -p zterm-cli --test daemon_autospawn --all-features
cargo +1.98.0 test -p zterm-daemon --test two_daemon_transport --all-features
sh tests/foundation/terminal-blackbox.sh --mode herdr
rustfmt +1.98.0 --edition 2024 --check crates/cli/src/terminal_ui/session.rs
just check
git diff --check
```

## Stop / rollback

If evidence requires wire changes, relaxing strict ACK, another state interpreter,
or application-specific logic, stop and revise the design. Do not hide a failure
with retries or an unconditional repaint. No publishing is included in this task's
initial scope; complete the agreed fixes and evidence before proposing a release.

## Execution record

Implementation and main-agent review are complete. See [validation.md](validation.md)
for exact checks, component/route evidence boundaries, real Herdr results and scoped
cleanup. The owning regression follows the observed full mode snapshot and deferred
resize, rather than fabricating a streamed mode-changing delta. See
[research/retrospective.md](research/retrospective.md) for the architectural/local
classification and why the earlier gates missed these defects.
