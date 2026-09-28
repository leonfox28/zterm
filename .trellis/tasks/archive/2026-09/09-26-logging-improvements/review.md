# Self-review, 2026-09-28

Requested by the user before committing. All changes remain on the existing
logging task and branch; no commit/archive is part of this review.

## Correction boundary

- Desktop retention: a missing/malformed control document or a one-off failed
  daemon registration currently loses the exact-socket capability marker. A new
  daemon is then mistaken for a legacy writer indefinitely, disabling rotation.
  `platform::diagnostics::Store` must retain the registration proven by the
  existing DaemonLock, repair its persisted marker on worker poll/write, and reject
  that knowledge for a different socket incarnation. Desktop composition must
  register the same Store used by its recorder. Expected files: platform and
  daemon diagnostics, existing lifecycle call sites and their regression owners.
- Android export: Compose `remember` loses the checkbox and launched-export
  snapshot on Activity recreation. Save both with the existing saved-state owner
  in `DiagnosticsSettings.kt`; test recreation and returned export selection in
  `SettingsUiTest.kt`.
- Android startup: `ContextImpl.getNoBackupFilesDir()` calls
  `ensurePrivateDirExists`, including exists/mkdir/chmod, synchronously (verified
  in installed Android 36 SDK sources). Defer the diagnostics directory lookup
  to `DiagnosticFiles` storage work, preserving early native installation. Test
  the Main/worker boundary in `DiagnosticsTest.kt`.

These are corrections to the approved retention, recreation and off-Main IO
contracts. They add no storage names, protocol/identity authority, public CLI or
UniFFI interface, collection policy, or unrelated refactor. Existing legacy
descriptor, process writer, unsafe-path and Android bridge tests remain required.

## Evidence and outcome

### Corrected logging findings

1. **Retention disabled after control damage or registration contention.** The
   new platform regression failed before correction on its actual file-size
   assertion. Store clones now share only the exact socket registration proof;
   the daemon composition registers the installed worker's Store. Polling and
   append repair missing/malformed/oversized markers under the stable writer
   lock. Polling also covers an idle daemon while independent CLI/updater
   writers remain active. A replacement socket does not inherit this proof.
   Six platform diagnostics tests pass, plus the child writer exercised by its
   parent. The isolated daemon composition regression passes too.
2. **Export opt-in lost during Activity recreation.** The existing Settings
   test first reproduced the checkbox resetting to OFF. A new real
   CreateDocument test then reproduced an export with `includes_debug=false`
   after the user selected detail and the stopped Activity was recreated. Both
   selection and launched-export snapshot now use `rememberSaveable`; the
   returned export retains the debug header and actual viewport event.
3. **Main-thread directory IO at logger installation.** Android 36 SDK
   `ContextImpl.java:877` calls `ensurePrivateDirExists` from its no-backup
   getter. DiagnosticFiles now resolves the directory lazily on storage access.
   An instrumented Context asserts that Main-thread construction/installation
   does not resolve it and the worker resolves it exactly once before writing.

Android debug and development-signed release each passed **7** diagnostics and
Settings tests on task-owned API36 arm64 AVD `Zterm_Logs_Review_0928`, serial
`emulator-5580`. This includes the real system picker, Activity recreation,
returned export contents, Main/worker boundary, caps/privacy, finite interval,
localized layout, renewal and picker cancellation. Debug lint/JVM and release
assemble/instrumentation/lint passed. Runtime test exports were deleted by their
fixture; the Downloads directory was confirmed empty.

Final workspace Clippy (`--all-targets --all-features -D warnings`) and
`git diff --check` passed after the retention poll correction; the installed
worker composition test was rerun and passed. The task-owned emulator was
stopped and its AVD deleted. No personal device/state or existing AVD was used.
The updated commit plan includes 79 paths after the approved Store fix;
the user has approved committing them and creating a PR.

The review also traced closed codec/export privacy, monotonic detail expiry,
queue/sink failure behavior, legacy descriptor cutover, updater/revocation
commit boundaries and shared-client event frequency. No additional logging
finding remains open within the reviewed scope.

Machine-local evidence uses `/tmp/zterm-logging-review-` plus these suffixes:
`retention-before.log`, `platform-final.log`, `composition-focused.log`,
`settings-before.log`, `picker-reproduction.log`, `debug-tests.log`,
`release-tests.log`, `android-check-final.log`, `release-build.log`.

The first real-picker fixture incorrectly called `ActivityScenario.recreate`
on a stopped Activity; that helper requires RESUMED. The corrected fixture calls
the real `Activity.recreate` on Main and waits for the original Activity's
destruction before selecting Save. A broad filtered Cargo command also reached
an unrelated custom black-box executable that rejects libtest filters; reran
the new composition owner correctly with `--lib`.

### Existing Store race found by the full gate (corrected and verified)

`just check` passed lint/policy and the logging owners, then failed
`store_handle_deadline_never_executes_expired_side_effects_and_shutdown_joins_once`
in `crates/daemon/tests/persistence.rs:503`: expected `DeadlineExceeded`, received
`OperationOutcomeUnknown`. The original `wait_for_store_response` in
`crates/daemon/src/store.rs` treated every failed `QUEUED -> EXPIRED` CAS as an
already-started command. If the actor itself set EXPIRED just before the waiter's
timeout, the CAS observes EXPIRED and incorrectly reports an unknown result.
No side effect ran in that state. At detection, both files were byte-for-byte
unchanged from HEAD in the failing area; diagnostics did not introduce this bug.

The concrete correction accepts `Ok(_) | Err(COMMAND_EXPIRED)` as
`DeadlineExceeded`, preserving `COMMAND_STARTED` as `OperationOutcomeUnknown`.
Its deterministic private-owner regression holds the reply sender open and
checks QUEUED, EXPIRED and STARTED at an elapsed deadline. This only requires
`store.rs` plus the existing spec/task artifacts. Original full-gate evidence:
`/tmp/zterm-logging-review-check.log`.

The existing persistence test passes when rerun alone
(`persistence-focused.log`), consistent with the scheduling-dependent race;
that does not resolve the demonstrated classification error or clear the gate.

The user approved including this correction: "好的，那你就一起改了".
Root-cause classification: **local implementation defect**. The existing
CommandGate already has one authoritative QUEUED/STARTED/EXPIRED state; the
disconnect branch classifies EXPIRED correctly, but the timeout branch conflates
it with STARTED. Fix `wait_for_store_response` in `crates/daemon/src/store.rs`
and add a deterministic regression in that file's existing private test owner.
Update the Store deadline contract in `effective-user-state.md` and this task's
scope/evidence/commit inventory. No database schema, actor/queue ownership,
deadline duration, retry policy, wire/API signature or logging policy change is
needed. Assert queued and actor-expired commands return DeadlineExceeded while
started commands still return OperationOutcomeUnknown; run the existing real
StoreActor persistence suite and full `just check` afterward.

The new `store::tests::timeout_preserves_queued_expired_and_started_classification`
regression failed on the original code with exactly the reported error-kind
mismatch, then passed after correction. It also verifies that the STARTED gate
is not revoked by the waiter. All 21 tests in the existing persistence suite
pass, including actual expired mutation rejection, response-disconnect outcomes,
bounded mailbox behavior and owner shutdown. Evidence:
`/tmp/zterm-logging-store-timeout-before.log`, `timeout-after.log` and
`persistence.log` with the same `/tmp/zterm-logging-store-` prefix.

The final **`just check` passed with exit 0** after the correction, including the
new regression, the previously failing persistence test, all workspace tests,
Clippy, formatting, docs, dependency checks, relay static checks and secret
scanning. Full evidence: `/tmp/zterm-logging-store-final-check.log`. No unresolved
review finding remains; code/spec/evidence are ready for the user-authorized commit step.
Android does not depend on the desktop Store module, so its already-passed
7-test debug/release runtime runs remain applicable without rebuilding fixtures.
