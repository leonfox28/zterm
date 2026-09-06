# Remote self-update: current behavior

## Evidence status and symptom

Initial diagnosis used source inspection; subsequent isolated real-PTY regression
evidence is recorded below. No real server or production installation was changed.
The user reports that accepting Session termination inside a remote zterm PTY
disconnects the connection without updating the installed executable.

## Causal chain

1. `crates/cli/src/lib.rs:863` directly calls `update_with_callbacks` from the
   foreground process and prints progress to its terminal.
2. `crates/daemon/src/operations.rs:467` validates the managed executable and
   prepares the signed candidate before daemon contact. At `:495` it obtains
   approval, then at `:498` awaits daemon stop in that same process.
3. `crates/daemon/src/session.rs:1016` ends each Session for daemon stop;
   `:2968` owns the close request and `:2998` calls the driver's explicit close.
4. `crates/platform/src/pty.rs:473` kills and reaps the root child. The updater
   is executing inside the terminal being ended, without independent lifetime.
   The exact signal/stdio failure on the user's host is still unverified.
5. Activation is later at `crates/daemon/src/operations.rs:520`; post-check and
   rollback start at `:534`, commit at `:557`, and configured startup at `:562`.
   Foreground termination can prevent reaching these steps.

## Root-cause classification and owner

**Architecture / boundary defect in update execution lifetime.** The operation
can terminate its own execution context before completing the destructive
transition. Existing confirmation and PTY shutdown contracts provide no
independent owner responsible for completing activation and startup.

Required invariant: after authenticated preparation, interruption approval and
successful handoff, ending the originating Session cannot terminate the update
owner. This belongs to local update/lifecycle orchestration, not transport.
Local self-attachment is an equivalent trigger and must share the correction.

## Existing mechanisms and boundaries

- `crates/platform/src/local_unix.rs:279` prepares detached daemon stdio/cwd;
  `:301` detaches the internal child using `setsid`. The launcher at
  `crates/daemon/src/lifecycle.rs:141` checks child readiness. These are existing
  OS primitives to evaluate, not authorization to make a second daemon service.
- `crates/daemon/src/operations.rs:505` acquires lifecycle ownership after stop
  and rejects a daemon restarted before activation. Preserve this and serialize
  competing update attempts correctly in the future design.
- `.trellis/spec/backend/distribution-lifecycle.md` remains the owner of
  candidate verification, activation, rollback, metadata, and new-daemon
  readiness. Reuse its implementation instead of duplicating a verifier.
- `docs/install.md:127` documents restart after configured update, no restoration
  of ended PTYs, and partial completion when a committed new binary cannot start.
- `.trellis/tasks/archive/2026-09/09-05-zterm-cli-commands-execution/prd.md`
  covers confirmation and startup. Current code/spec supersede the older
  distribution-task statement that update must not restart the daemon.

## Confirmed product scope and design direction

On 2026-09-06 the user explicitly permitted ending existing Sessions. Preserve
the existing shutdown impact, with manual reconnect after configured startup.
The user also asked whether this is architectural or implementation-only: the
classification above concerns the updater execution lifetime, not the whole
remote terminal architecture.

An explicitly invoked updater must survive the original PTY; handoff must be
acknowledged before stopping the daemon. Preserve the single prepared candidate
and single activation owner. Frontend loss after acceptance must not cancel the
update, and final outcome must remain observable after disconnect.

Do not treat reconnect changes, arbitrary sleeps, shell `nohup`, or moving the
update into the daemon being stopped as a complete lifetime contract.

Session persistence across daemon replacement and changes to related
destructive commands are outside this task's confirmed scope.

## Minimal meaningful regression

- Run a real PTY inside an isolated daemon fixture, with explicit test paths
  and installation state. Do not use the developer's real daemon or binary.
- Invoke the production update flow inside that PTY with authenticated release
  fixtures. Confirm shutdown, then independently observe target activation,
  target-version daemon readiness, and accurate outcome after PTY termination.
- Prove the regression fails before the fix. Cover local self-attachment and
  preserve ordinary external-terminal completion/exit behavior.
- Add focused cancellation / handoff-failure / frontend-loss coverage, and reuse
  existing release verification and activation rollback tests.
- Final real remote acceptance requires a signed installation that already
  contains the fixed updater; downloading a new candidate does not change the
  old updater's execution logic during the initial migration.

## Resolved technical direction

- `design.md` makes the detached child prepare and own the candidate from the
  beginning, eliminating transfer of trusted staging and a second download.
- A private parent/child channel preserves foreground confirmation and truthful
  results. Its acceptance boundary cannot bypass late Session impact approval.
- Existing daemon.log / zterm logs provide configured-update diagnosis; reopen
  after startup rotation rather than inventing persistent update job state.
- Existing distribution fixtures use a map fetcher and fixture signing key but
  need an executable candidate role for startup. `implement.md` assigns one
  real PTY/activation regression and retains the production official-build guard.
- A real signed remote run remains dependent on a released fixed updater and
  newer signed target; this limitation must remain distinct from test results.

## Executed baseline regression (2026-09-06)

`cargo +1.98.0 test -p zterm-daemon --lib update::tests::update_completes_after_its_originating_pty_ends -- --nocapture`
failed before switching to detached execution. The fixture used the production
Session/PTY shutdown and factored activation operation with a candidate prepared
through the existing signed-manifest/archive verifier. It observed
`approved=true`, `original_still_installed=true`, and no foreground-result file
after the 12-second deadline. The old daemon had closed the originating PTY.
The test does not fabricate a replacement activation or infer success from a
spawn marker. This confirms the missing updater lifetime boundary independently
of remote transport; the exact OS exit signal is not claimed by this observation.

## Regression result after correction

The same `update_completes_after_its_originating_pty_ends` test now passes. It
observes the installed fixture executable change, target daemon readiness at
9.1.0, zero retained Sessions, preserved identity bytes, one preparation, staging
cleanup and success in the existing log after the original foreground ends.
The fixture version override exists only under cfg(test) and is not compiled
into released daemon services.

## Bug retrospective

- Root cause: cross-layer execution-lifetime contract plus an implicit
  assumption that the updater runs outside affected PTYs. Existing unit tests
  covered verification and startup helpers without this ownership composition.
- Prevention: one update execution owner for all origins, with an independent
  process lifetime; the same regression fails when ownership returns to the PTY.
- Related commands such as restart/reset/uninstall may have their own caller
  lifetime concerns. They remain outside this task; no remote-specific special
  case or generic service framework was added here.
- Updated executable contracts: distribution-lifecycle, logging-guidelines,
  local-daemon-ipc, and a short cross-layer guide trigger. This repository has
  no `src/templates/markdown/spec` tree to synchronize.
