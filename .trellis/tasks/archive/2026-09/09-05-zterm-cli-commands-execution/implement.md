# Implementation plan: Zterm CLI lifecycle UX

Status: expanded plan for R3–R20. C1–C5 and C6's -n shorthand are selected;
logs -f/--follow has been explicitly removed. Log enrichment uses existing
recording infrastructure. Implementation, validation and the approved work commit are complete; task bookkeeping follows.

## Entry gate

- [x] Command inventory and targeted lifecycle/output research recorded.
- [x] PRD revised after user selected English prompts, -y and force removal.
- [x] Incorporate selected output/setup/pairing conveniences and remove follow.
- [x] Technical design defines owners, startup behavior and validation.
- [x] Real implement/check JSONL context manifests curated.
- [x] User authorized all selected changes for this round; the latest refinement
  only removes logs follow. Continue within that existing authorization.

Use one native trellis-implement worker for this coherent slice, then one
independent native trellis-check worker. The main session coordinates changes
and owns final task/spec/commit flow. Every dispatch begins with the exact
`Active task: .trellis/tasks/09-05-zterm-cli-commands-execution` header, uses
native context injection with child-side loading fallback, and gives explicit
file ownership. Workers are not alone in the codebase and must preserve others'
edits. No external provider CLI or additional review fan-out is needed.

## Ordered work

### 1. Honor shutdown admission at the Session owner

- [x] Read the relevant Session shutdown/admission implementation and tests.
- [x] Change `crates/daemon/src/session.rs` to atomically reject unapproved
  shutdown with active or admitted Session ownership, preserving current
  bounded cleanup on approved/idle shutdown.
- [x] Wire existing LocalStopRequest.force and LocalStopResponse.stopping through
  `crates/daemon/src/service.rs` and `crates/daemon/src/client/ipc.rs`.
- [x] Extend existing Session/IPC tests for retained live work, creation
  admission race, idle stop, and explicitly approved cleanup. Update existing
  unconditional teardown callers to pass explicit true where needed.
- [x] Keep protobuf schema and wire-major unchanged.

### 2. Integrate confirmation and update startup

- [x] Add the shared typed confirmation boundary in
  `crates/daemon/src/operations.rs`; keep terminal I/O in the CLI callback.
- [x] Route stop/restart/update through it; unapproved newly admitted work
  returns to the same invocation's confirmation flow.
- [x] Keep authenticated PreparedRelease alive across confirmation without
  duplicate downloads, stop-before-verification, or lifecycle locks over stdin.
- [x] After successful activation/commit and lock release, ensure the new daemon
  and compare readiness to the authenticated installed build identity.
- [x] Implement honest success, unconfigured setup guidance, and post-commit
  startup-failure results. Retain existing activation rollback behavior.
- [x] Expose concise typed update progress at actual phase boundaries for CLI
  English rendering; do not create a separate update log writer or journal.
- [x] Add focused runtime tests using existing test-private launch/activation
  fixtures; preserve managed-release validation in production.

### 3. Apply public CLI contract

- [x] In `crates/cli/src/lib.rs`, remove five JSON options and unused rendering
  types; simplify text dispatch and remove obsolete dependencies only if unused.
- [x] Give all seven lifecycle/destructive commands -y/--yes and shared English
  confirmation. Remove public force from update/stop/restart/reset/uninstall;
  reset/uninstall show Session and deletion impact in one confirmation.
- [x] Accept y/yes case-insensitively; default to cancel and never read input
  with explicit yes or empty impact.
- [x] Adapt `crates/cli/tests/command_side_effects.rs` and
  `crates/cli/tests/daemon_autospawn.rs` away from public JSON assertions.
- [x] Extend the existing isolated PTY harness for one actual English prompt
  and accepted continuation; cover cancellation and bypass at the shared
  callback boundary without multiplying equivalent process scenarios.
- [x] Verify hidden self-check remains consumed and absent from public help.
- [x] Render names-first device/Session tables and actionable empty results;
  keep full IDs, directional trust and truthful observed connection labels.
  Move detailed network diagnostics from status into doctor without losing
  them or changing their no-autospawn behavior.
- [x] Default first setup to official-n0 without a profile prompt/required flag;
  preserve explicit self-hosted configuration and repeated-setup identity.
- [x] Rename pair accept --name to --alias, add direction-aware success and
  safely shell-quoted connect hints, add ticket TTL/receiver guidance on stderr,
  and fix the invalid 30s help example. Preserve one ticket emission on stdout.
- [x] Default only session list's optional target to local.
- [x] Add logs -n and useful empty-log output using the existing bounded tail.
  Do not add -f/--follow or any continuous reader scaffolding/tests.

### 4. Enrich existing daemon event logging

- [x] Use research/logging-gap-analysis.md to select actual committed event
  owners in lifecycle/session/network/connection_broker/pairing/service.
- [x] Add readable English INFO transitions and classified WARN/ERROR outcomes
  with version/PID, safe Session correlation and stable reason fields.
- [x] Keep events at their authoritative owner so retries/adapter paths do not
  duplicate create/end events. State refreshes, normal detach, terminal frames
  and RTT samples must not generate warning/volume spam.
- [x] Reuse the existing tracing subscriber/writer and managed file inventory;
  retain startup-only rotation. No new recorder, upload or retention service.
- [x] Capture real lifecycle/state transitions with an isolated test subscriber;
  verify useful fields and sentinel-content exclusion. Do not test by changing
  global logging configuration for concurrently running unrelated tests.

### 5. Update user docs and executable contracts

- [x] Update `README.md`, `docs/remote-cli.md`, `docs/core-local-daemon.md`,
  `docs/install.md` and any directly affected current documentation.
- [x] Update `.trellis/spec/backend/local-daemon-ipc.md`,
  `.trellis/spec/backend/distribution-lifecycle.md` and
  `.trellis/spec/backend/session-service.md` with the approved behavior.
- [x] Fill `.trellis/spec/backend/logging-guidelines.md` with the implemented
  logging contracts and update setup/pairing docs/specs where their old defaults
  or parameter names conflict with the selected UX.
- [x] Search current product/help/docs/tests for stale JSON/force/stopped-after-
  update claims. Preserve unrelated commands, internal wire fields, gh options,
  and archived history. Do not make a repository-wide textual replacement.

## Checks and review

Run relevant focused gates after their owners change:

```sh
cargo +1.98.0 test -p zterm-daemon --lib
cargo +1.98.0 test -p zterm-daemon --test local_session_ipc
cargo +1.98.0 test -p zterm-cli
```

These are local deterministic gates; do not execute real macOS Iroh cases.
After integration, run the repository's native quality gate once:

```sh
just check
git diff --check
```

- [x] The independent checker maps final code to PRD R3–R20 and fixes verified
  drift/failures, focusing on empty-impact races, interactive EOF, stale public
  options, new-binary readiness/startup partial failure, consolidated destructive
  approval, default setup identity stability, truthful tables and safe log events.
- [x] Review final changed files and native gate evidence. Repeat checks only
  for checker changes or an actual unresolved failure. Record hosted limitations
  rather than building substitute release or network infrastructure.
- [x] Work commit approved and created. The existing finish-work scripts now
  archive this task and record its journal before release preparation.

## Risk / rollback points

- The Session shutdown admission change must remain atomic with create and keep
  cleanup owners visible. Roll back an incomplete behavioral change as a unit,
  not just its corresponding tests.
- Update retains old executable backup only through the existing activation
  transaction. After successful commit, startup failure leaves the new binary
  installed and returns a nonzero partial-completion diagnostic.
- Never test using real `~/.zterm`, uninstall/update the user's executable, or
  end user Sessions. State/executable fixtures must be task-private.
- No live deployment, publication, version bump, or release is part of this
  implementation approval.
