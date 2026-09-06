# Implementation verification

## Behavior evidence

- Before the ownership fix, the actual daemon-owned PTY regression failed:
  approval was recorded, the original executable remained, and the originating
  updater never wrote a final result after Session shutdown.
- After the fix, the same scenario observes the authenticated fixture candidate
  replace the executable, the 9.1.0 fixture daemon reach readiness, old Sessions
  end, identity bytes remain unchanged, exactly one preparation occur, and the
  surviving update owner record success.
- `cargo +1.98.0 test -p zterm-daemon --lib update::tests -- --nocapture`:
  **14 passed**, including the fixture child entry. Checks cover foreground
  termination, external-terminal completion, -y, cancellation/staging cleanup,
  pre-handoff EOF, failed launch, late Session admission without approval,
  stalled/partial control writes, accepted-channel unknown outcomes, source
  replacement, activation rollback, startup partial completion, pre-setup state
  preservation, and final logging after daemon startup rotation.
- CLI coverage verifies the internal updater is hidden, exclusive, and rejected
  alongside normal commands. Existing CLI confirmation tests own y/yes/EOF and
  noninteractive input policy; no alternate updater prompt parser was added.
- Existing distribution tests still own signature, target, archive, version,
  and development-build refusal. No test key or source override enters the
  production executable. The fixture's daemon version field is cfg(test) only.

## Quality gates

- Initial full `just check`: passed on the current macOS arm64 host, including
  policy, Clippy, workspace tests, docs, dependency checks, relay-probe checks,
  upstream archive verification and available Docker Compose static checks.
- Final review corrected accepted-channel failures to use
  `operation_outcome_unknown` rather than `cancelled`. Subsequent targeted tests
  and workspace Clippy passed. The final serial `just check` also passed with the correction and all
  process tests, including the new unknown-outcome test.
- One focused process-test run launched while other gate/build work was active
  had child startup/control failures. A diagnostic single regression and the
  subsequent complete 14-test suite passed. The original failure did not retain
  a child diagnostic, so its exact environmental cause is unproven. No product
  timeout increase or retry was added to hide it. Final process tests are run
  without overlapping builds of their re-entered test executable.

## Evidence boundaries

- The regression uses real Unix processes, PTYs, same-UID IPC, Session cleanup,
  filesystem activation and daemon startup, with isolated signed fixture inputs.
  It does not exercise a real remote Iroh connection or an official signed
  Release on a user's server.
- Linux arm64/x64 runtime evidence remains assigned to the existing hosted
  native jobs. macOS execution is not a claim of Linux execution.
- Real-server acceptance needs an installed published updater containing this
  fix plus a newer signed compatible target. Bootstrap the first fixed updater
  via SSH; then invoke the subsequent update inside a remote zterm Session,
  approve interruption, reconnect manually, and check executable/daemon versions,
  identity/pairing retention and the final `zterm logs` outcome.
- No server, production daemon, identity, installed executable, signing material,
  release or remote repository was mutated by this implementation session.

## Commit status

Work commit approval and task archival are pending. Keep the task active until
the user approves the commit and remaining real-release acceptance is handled
or assigned to the release verification task.
