# Remote self-update implementation plan

Status: implementation, isolated regression coverage and final quality review
complete; v0.1.24 is published and real-server acceptance remains pending. The user approved the final plan and implementation on
2026-09-06 and subsequently authorized the work commit and release workflow.
Task start used `--allow-empty-context` for the authorized inline mode.

This is one coherent deliverable with one activation owner and end-to-end
acceptance. Keep one task; there are no independently shippable child tasks.
Codex inline mode: the main session implements and checks directly.

## 1. Load execution context

- [x] After explicit approval of the final plan, activate this task and load
  `trellis-before-dev` and Phase 2.1.
- [x] Read `prd.md`, `design.md`, and `research/current-behavior.md`.
- [x] Load backend distribution-lifecycle, local-daemon-ipc, pty-lifecycle,
  effective-user-state and logging guidelines; read the cross-layer and
  evidence-driven simplicity guides for the new private process boundary.

## 2. Establish the failing lifetime regression

- [x] Reuse daemon/PTY and distribution fixtures; add an executable fixture
  role for signed-candidate identity/self-check and test-daemon startup.
- [x] Run the existing update orchestration within a real daemon-owned PTY.
  From an independent observer, assert activation and target readiness after
  the Session ends. Record the pre-fix failure and exact termination evidence.
- [x] Keep production guard tests proving that ordinary development builds
  cannot update or touch live installations. All fixture paths stay temporary.

## 3. Implement the one-shot update owner

- [x] Add platform-owned private channel / child detachment primitives using
  existing Unix process conventions. Require ready only after detachment;
  close inherited unused endpoints and keep PTY descriptors out of the child.
- [x] Add the mutually exclusive hidden CLI entry and updater module. Keep
  production effective-account paths and the official managed-build guard.
- [x] Move the current update body behind one internal execution boundary;
  make `update_with_callbacks` the foreground adapter for every public update.
  The child creates/owns the single PreparedRelease through final completion.
- [x] Implement bounded typed messages for preparation, Session confirmation,
  continue/acceptance, progress and final results. Preserve `-y`, cancellation,
  EOF, noninteractive refusal and late Session admission confirmation.
- [x] Prove frontend loss after acceptance cannot cancel or backpressure the
  mutation owner; loss before required approval cannot authorize termination.
- [x] Reuse the existing stop/activation/rollback/startup sequence and lifecycle
  lock. Under configured activation ownership reject a superseded source
  executable or restarted daemon, without adding a job/lock subsystem.

## 4. Complete user-visible outcome and diagnostics

- [x] Display the verified target, Session impact, connection interruption,
  continued-update behavior, manual reconnect and `zterm logs` guidance before
  allowing destructive execution.
- [x] External terminals wait for true completion and preserve exit status.
  Spawn/acceptance acknowledgements must never print final success.
- [x] Configured updates write bounded stage/outcome records to existing
  validated daemon.log; reopen after daemon log rotation. Preserve pre-setup
  no-state-creation behavior and avoid sensitive log fields.
- [x] Explain activation rollback versus committed-but-not-started results
  accurately; loss of logs/output cannot trigger a second rollback engine.

## 5. Focused verification and acceptance mapping

| Criteria | Authoritative evidence |
| --- | --- |
| A1 / A2 | Real daemon-owned PTY update regression with independent version/readiness observer; local self-attachment and external-terminal variants through the same owner |
| A3 | Foreground/control tests: cancel, EOF, unapproved noninteractive invocation, failed child readiness/handoff, lost frontend at approval and after acceptance |
| A4 | Existing distribution signature/version/candidate tests; fixture fetch count proves a single preparation |
| A5 | Existing activation rollback and startup-partial-completion tests composed with child results/logs; final result remains in current log after rotation |
| Shared lifecycle | Late Session admission without approval, source executable superseded while waiting, and restarted-daemon rejection |

Candidate commands after the corresponding tests exist:

```sh
cargo test -p zterm-daemon --lib
cargo test -p zterm-daemon --test detached_lifecycle
cargo test -p zterm-daemon --lib update::tests
cargo test -p zterm-cli --lib --bins
cargo test -p zterm-cli --test command_side_effects
cargo test -p zterm-cli --test daemon_autospawn
cargo test -p zterm-platform --lib
```

The process regression lives in `crates/daemon/src/update/tests.rs` and re-enters
one exact unit-test child function. Keeping it under cfg(test) allows the existing
private signed-fixture verifier and a daemon version override without adding a
production test switch, a new public injection API or another Cargo harness.
The test still executes real child processes, PTYs, daemon IPC and activation.
Use the repository's canonical `just` checks for the final quality gate.

Canonical commands observed during planning:

```sh
just check-fast
just ci-policy
just ci-unix false true
just check
```

Use `check-fast` while iterating and the required final `check` once, accounting
for checks it already includes. The explicit ci-policy / ci-unix commands are
the owning reproduction commands, not instructions to repeat a green full gate.
The `false true` Unix arguments match the current macOS docs/smoke assignment;
Linux x64 owns `true true`, and Linux arm64 owns `false false`.

## 6. Documentation, review and handoff

- [x] Update `docs/install.md` and relevant CLI docs with interruption,
  completion/diagnostics, manual reconnect, and one-time migration via SSH.
- [x] Update distribution-lifecycle and logging specs to capture the new owner
  and narrowly justified updater logging; update local-daemon-ipc if its
  confirmation contract wording changes. No remote wire/schema change.
- [x] Run `trellis-check` inline, focused regression evidence, and the canonical
  policy/native checks required by changed files. Record platform limits.
- [x] Record external signed-release acceptance prerequisites and exact steps:
  bootstrap fixed updater via SSH; connect remotely; invoke update to a newer
  signed compatible version; confirm Session shutdown; wait for daemon readiness;
  manually reconnect and verify version, identity and pairing.
- [x] Keep isolated-test evidence separate from a real signed/server run. Do
  not claim full remote release acceptance until that run occurs.
- [x] Follow the authorized repository commit/release workflow for v0.1.24.
  Record PR, exact main CI, tag and immutable publication evidence; keep real
  server acceptance separate from publication.

## Risk and rollback points

- Process entry / descriptor inheritance: failure must occur before daemon
  stop; test that child readiness reflects actual isolation.
- Confirmation/control: acceptance and interruption authorization are different
  facts. Cover idle-preflight/new-Session races and missing foreground input.
- Candidate/lifecycle refactor: keep one verifier and one activation body;
  avoid moving trust into serialized candidate metadata or duplicating rollback.
- Log rotation: reopening the current managed path is essential for final
  results to be visible via the existing bounded reader.
- Tests: reuse existing fixtures; no task-private replacement daemon stack or
  insecure production build switch to make signed-flow tests convenient.
- If the real regression contradicts the recorded root cause, return to planning
  with the evidence before broadening ownership changes.

## Current evidence / remaining external acceptance

- The real PTY regression failed on the retained foreground execution path
  (`approved=true`, original executable retained, updater result absent), then
  passed after routing the same operation through the detached worker.
- The full `just check` gate passed on this macOS host. A later local correction
  changes lost-after-handoff control errors to `operation_outcome_unknown` rather
  than `cancelled`; focused tests, final Clippy and a final serial `just check` verify that correction.
- PR #30 merged and v0.1.24 was published through the authorized release
  workflow. Three-platform hosted tests, protected signing, final installer
  verification and immutable publication all passed; see `research/release.md`.
  No user server, production installation, identity or Session was modified.
  Full real-server acceptance remains pending an installed fixed updater and
  a newer signed compatible target, as planned.
- The user authorized the proposed work commit and release workflow. Keep the
  task active while the outstanding real signed remote acceptance remains
  pending; publication alone does not establish that acceptance.
