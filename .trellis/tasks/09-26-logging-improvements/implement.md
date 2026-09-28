# Implementation plan: comprehensive local logging

Status: in progress; final plan explicitly approved with "开始吧" on 2026-09-28.
Mode: Codex inline. Main session implements/checks directly; no implement/check agents.
Sources: `prd.md`, `design.md`, `research/logging-audit.md`.

## Task structure and dependencies

Keep one coherent task with ordered slices: event/storage contract, desktop
coverage/controls, Android composition, then integrated validation. The common
schema, migration and exporter form one compatibility boundary; splitting these
into separately activated/released tasks would leave interim logging contracts.
The slices below are implementation ordering, not independent release claims.

## 0. Review and activation

- [x] User reviews scope, 15-minute interval, separate retention limits,
  commands/export and compatibility limits in the final summary.
- [x] PRD convergence pass has no unresolved product questions or lost G1–G9
  evidence/LOG requirement mapping.
- [x] After a subsequent explicit approval, run:
  `python3 .trellis/scripts/task.py start .trellis/tasks/09-26-logging-improvements`.
- [x] Load `trellis-before-dev`; read logging, effective-user-state,
  local-daemon-IPC, shared-client, Android app, distribution and relevant
  thinking guides. Inline mode does not need JSONL dispatch manifests.

## 1. Shared contract and rotating storage (LOG-1/4/6/7)

- [x] Add lightweight diagnostics library and workspace wiring. Define typed
  event envelope, safe context, levels, JSONL codec and text renderer.
- [x] Implement default/detail gates, finite interval, bounded key/detail
  dispatch, suppression counters and injectable sinks/clocks.
- [x] Add desktop private-file sink using platform secure-open/lock primitives,
  two independent rotating lanes and snapshot-based bounded reads.
- [x] Update managed inventory, control-state handling, legacy migration and
  reset/uninstall evidence. Handle old-running-daemon compatibility explicitly.
- [x] Replace inherited managed stdout/stderr writers and the two hand-written
  recorders together. Add content-free startup/fatal/panic outcomes and bounded
  shutdown flushing; preserve updater preflight and durable barriers.
- [x] Verify runtime caps, concurrent processes, safe paths, expired controls,
  lock contention/disk failure and zero-state inspection in task-private roots.

Primary files: new `crates/diagnostics/**`, workspace Cargo manifests/lock,
`crates/platform/src/user_state.rs`, `local_unix.rs`, new logging storage module,
`crates/daemon/src/lifecycle.rs`, `connection_progress.rs`, `update.rs`.
Do not leave a build where one writer still bypasses rotation coordination.

## 2. Desktop and shared-client event coverage (LOG-1/2/6/7)

Depends on slice 1's event/sink contract.

- [x] Migrate daemon/Session/network/pairing events without changing commit,
  lease or replay behavior. Add safe Session rename outcomes.
- [x] Record classified attachment, service, admission and upload outcomes at
  owners; distinguish expected cancellation and rate-limit repeated rejection.
- [x] Split durable authorization revoke from later cleanup result; link
  adapter completion without repeating committed events.
- [x] Persist configured-user update preparation failures and correct severity
  of failed/partial outcomes. Preserve no-setup behavior and independent worker.
- [x] Carry safe process/local operation/connection context through retries;
  add known Session/attachment context without credential-bearing IDs.
- [x] Cover shared connect/reconnect, route class, resync and input/presentation
  state transitions. Keep startup observer retirement and first-Active meaning.
- [x] Exercise real owner paths, replay/deduplication, sentinel redaction and
  post-commit failure; no hard-coded formatter-only coverage as the main proof.

Primary files: `crates/daemon/src/{session,session_wire,service,pairing,pairing_service,
network,connection_broker,local_ipc,update}.rs`, `session_wire/upload.rs`,
`crates/client/src/{iroh_controller,session,view,upload,progress}.rs`,
`crates/cli/src/main.rs`, `terminal_ui.rs`, `terminal_ui/session.rs`.

## 3. Desktop inspection, control and export (LOG-4/5/6)

Depends on slice 1; use events from slice 2 for integration evidence.

- [x] Preserve `logs -n/--lines`, defaults, English empty behavior and zero lines.
- [x] Add the reviewed filters, `--json`, archive/detail inclusion and strictly
  bounded reads under concurrent append/rotation.
- [x] Add `logs debug on|off|status`, finite renewal/expiry and no-autospawn
  behavior. Confirm a running daemon observes policy without a restart.
- [x] Add explicit no-overwrite streaming export with safe metadata and omitted
  legacy counters. Exclude private config/status/identity and remote logs.
- [x] Verify setup missing, daemon stopped, legacy text, invalid control state,
  existing output destination, cancellation and writer failure behavior.

Primary files: `crates/daemon/src/operations.rs`, `crates/cli/src/lib.rs`,
`crates/cli/tests/daemon_autospawn.rs`; new diagnostics integration fixtures.

## 4. Android release composition and Settings (LOG-3/5/6/7)

Depends on slice 1's schema/gates and slice 2's shared client events. Android
does not depend on the desktop storage adapter or daemon.

- [x] Add narrow UniFFI sink/control/event methods; regenerate bindings with
  existing tools, never edit generated Kotlin.
- [x] Install the logger from Application before identity/network setup; add
  app-private no-backup rotating storage and persisted finite interval.
- [x] Route safe Kotlin lifecycle/network/operation/update/storage/notification
  outcomes to the shared encoder and native client events to the same sink.
- [x] Keep all disk/export work off Main and terminal actors; no logger closure
  on Activity recreation/backgrounding and no recursive logging callback.
- [x] Add localized Diagnostics Settings controls, time remaining and export
  through the system create-document flow, independent of successful pairing.
- [x] Validate debug and release behavior, Kotlin/native correlation, expiry,
  process restart, bounded persistence and successful export/cancel/error paths.

Primary files: `crates/android/src/lib.rs`, `network.rs`, `terminal.rs`, new
diagnostics bridge module; Android `ZtermApplication.kt`, `AppRepository.kt`,
`AppUi.kt`, new diagnostics store/UI, lifecycle/network/update/notification
owners, localized resources and focused tests.

## 5. Validation, specs and completion

Use each real operation/contract's existing test owner. Add focused regressions
for new cross-boundary behavior rather than duplicating the full test matrix.

| Behavior | Authoritative evidence |
| --- | --- |
| Codec/filter/detail expiry | Isolated diagnostics tests with injected time and bounded records |
| Multi-process rotation/read bounds | Platform/daemon process fixtures using private roots, including an updater across rotation |
| Commit/replay/error truth | Existing Session, pairing and update owner fixtures; revoke commit then cleanup failure |
| CLI no-autospawn, migration, controls/export | CLI process fixtures; concurrent append and archive selection |
| Android persistence and interval | Store/native bridge fixtures, both debug and release artifacts |
| User export and lifecycle | Selected disposable emulator; recreate Activity/restart app and exercise system picker |
| Terminal responsiveness/quiet default | Owner event-count assertions for healthy repeated frames and a targeted PTY/UI integration scenario |
| Remote recovery | Existing disposable Linux real-Iroh fixture; no live macOS network daemon or personal host state |

Focused commands once implementation exists:

```sh
cargo +1.98.0 test -p zterm-diagnostics
cargo +1.98.0 test -p zterm-platform
cargo +1.98.0 test -p zterm-daemon operational_logs
cargo +1.98.0 test -p zterm-daemon configured_progress_logs
cargo +1.98.0 test -p zterm-daemon update::tests
cargo +1.98.0 test -p zterm-daemon --test pairing_secrets
cargo +1.98.0 test -p zterm-cli --test daemon_autospawn
sh tests/terminal-dependency-policy.sh
just android-check
sh tools/android/build.sh :app:assembleDebug :app:assembleRelease :app:lintRelease
```

Add newly named targeted tests for bounds, failure outcomes and exports to this
list during implementation. Select the emulator serial before instrumentation;
run only diagnostics/affected Settings bridge tests first using the existing
`connectedDebugAndroidTest` runner and class filter. A release build alone is
not release-runtime evidence: install a signed testable release artifact on a
disposable selected emulator and verify collection/restart/export there too.
Do not use production phone/account state or claim unavailable runtime evidence.

- [x] Load `trellis-check` and run required checks; finish with `just check` and
  affected Android gates. Repeat only for later changes or unresolved failures.
- [x] Update logging, effective-user-state, shared-client, Android and lifecycle
  specs to the implemented contracts; document commands/limits/migration in
  README, desktop/Android docs. Use `trellis-update-spec`.
- [x] Record outcomes and platform limits in `verification.md`.
- [ ] Only when approved scope passes, continue Trellis finish/commit workflow;
  never archive this task on the strength of planning documents alone.

## Rollback and review points

- Highest-risk boundary: changing all desktop writers while introducing runtime
  rotation. Review every sink and old/new updater handoff before widening events.
- Preserve terminal/Session authority and protocol compatibility. Logging IDs
  are metadata, never authorization inputs or grounds for replay.
- Do not remove old logs during planning or tests. Migration is bounded and
  local; older cleanup tools may reject new filenames, as documented in design.
- Export does not guarantee a crash-proof audit trail; loss counters and typed
  failures must not pretend that a failed sink recorded an event successfully.
- Material product changes from this plan return to planning review before
  implementation proceeds with those changes.

## Implementation boundary

The gap is missing persistent client diagnostics and unbounded, incompatible
desktop writers. The new shared crate owns encoding/admission, platform owns
secure storage, and existing operation owners emit committed outcomes. Files
are listed per slice above. No protocol/state authority changes, transcripts,
telemetry or unrelated refactor. Existing lifecycle/replay tests guard behavior.

## Implementation checkpoint (2026-09-28)

All implementation slices, product acceptance criteria and required local
quality gates are complete. Final `just check` passed after all regression fixes;
Android release assemble/lint and debug lint/JVM gates passed. The latest
installed release passed diagnostics/Settings, real remote reconnect, process
restart and actual system-document export. Detailed evidence and prior fixture
failures are recorded in `verification.md`; all task-owned runtime fixtures have
been cleaned up.

Phase 3.3 spec sync is complete. The user approved the concrete commit plan
and PR creation on 2026-09-28. Continue with the work commit, task archive and
session journal, then push `feat/logging-improvements` and open a PR to `main`.

## User-requested second review (2026-09-28)

Three logging defects were reproduced/verified and corrected: retention marker
recovery, saved export selection and Android startup directory IO. See
`review.md`. Focused storage/composition tests and 7 tests per installed Android
variant pass. The user approved including the existing Store deadline race found
by the full gate. Its deterministic regression failed before correction and now
passes, together with all 21 persistence tests. The existing timeout branch now
distinguishes EXPIRED from STARTED. Final `just check` passed with exit 0 after
all review corrections. The Store contract and 79-path commit plan are updated;
no implementation or review finding remains open. Phase 3.4 is authorized by
the user, including the following branch push and PR creation.
