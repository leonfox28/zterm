# Logging implementation verification

Date: 2026-09-28. Branch: `feat/logging-improvements`, based on `acecb45`.
Implementation and checks were performed inline. No personal daemon, phone,
identity, address book or deployed log files were used.

**Current review status:** the user-requested second review corrected three
logging defects; debug/release each passed 7 runtime tests. The user approved
including the existing Store deadline race found by the full gate. Its
deterministic regression failed before correction and now passes, as do all
21 persistence tests. The final `just check` passed with exit 0 after correction
(`/tmp/zterm-logging-store-final-check.log`). No review finding remains open.
See [review.md](review.md) for before/after evidence and the approved boundary.

## Delivered behavior

- Shared schema-1 JSONL, safe process/operation/connection/Session/attachment
  correlation, classified outcomes and elapsed time. Existing owners retain
  mutation, replay, lease and terminal authority.
- Default key events plus explicit 15-minute detail enable/renew/off. Separate
  bounded queues and storage lanes; producer admission performs no filesystem IO.
- Coordinated desktop runtime rotation (16 MiB total), stable writer lock,
  legacy-daemon cutover, safe path validation and reset/uninstall inventory.
- CLI bounded tail, level/component/Session/time filters, archive/detail/JSON
  selection, local controls and no-overwrite streaming export.
- Android Application-owned Rust/Kotlin diagnostics in debug and release,
  private no-backup storage (8 MiB total), localized Settings and CreateDocument
  export available independently of identity/network initialization.
- Safe startup/fatal, operation, upload/update, revocation/cleanup and ongoing
  reconnect/route/frontend events. No terminal contents, credentials, arbitrary
  dependency records, telemetry or automatic uploads.

## Checks and evidence

| Check | Result / assertions |
| --- | --- |
| `just check` | Final run passed with exit 0 after all logging review corrections and the approved Store fix. Includes policy, formatting, workspace Clippy/tests/docs, dependency licenses/advisories, relay static checks and secret scanning. The earlier failing run is retained as reproduction evidence in `review.md`. |
| Store timeout regression and persistence suite | Deterministic QUEUED/EXPIRED/STARTED regression passes after failing before the correction; all 21 persistence tests pass, including expired mutation rejection and response-loss ambiguity. |
| `cargo +1.98.0 test -p zterm-diagnostics` | 7 passed: closed codec/privacy, explicit detail/correlation, interval validation, blocked sink/queue isolation, recovered losses, fractional timestamp merge, control failure/clock rollback without renewal. |
| Platform diagnostics fixtures | 6 passed plus a child-process helper invoked by its parent: concurrent thread/process rotation, stable lock inode, snapshot handles, safe paths/export, oversized legacy tails, exact legacy socket cutover and recovery after control loss/registration contention. |
| Managed cleanup fixtures | 3 passed after extending the inventory to all six files and exact atomic-write residues; unknown files and unsafe nodes remain protected. |
| Daemon owner fixtures | Session create/replay/detach/network counts and content sentinels pass. Configured startup progress preserves rotation and observer retirement. |
| Update fixtures | 15 passed, including configured-user preparation failure, pre-setup no-state behavior and durable activation followed by partial completion. |
| Revoke IPC fixture | Actual durable authorization commit followed by forced cleanup failure emits linked committed and partial-completion records; generation remains committed. |
| Shared client recovery | Actual tunneled reconnect preserves Session, replaces attachment, shares operation/timing and emits one route event for 100 unchanged-path RTT samples. |
| Upload owner fixtures | 4 passed after cancellation classification changes, including staging cleanup while the terminal survives and committed publication boundaries. |
| CLI process fixtures | `daemon_autospawn` and `command_side_effects` pass: no inspection autospawn/setup state, bounded legacy/current/archive tail, controls, JSON and no-overwrite export. |
| Linux real Iroh | `connection_broker`: 2 passed; `two_daemon_transport`: 2 passed, zero ignored, in the task-owned Linux VM/container. |
| Android static/build | Final release assemble + release instrumentation assemble + `lintRelease` passed; final `just android-check` (debug lint/JVM tests) passed. |
| Android debug runtime | 7 focused diagnostics/Settings instrumentation tests passed after review corrections on the explicitly selected API36 arm64 emulator. |
| Android release runtime | Latest locally signed testable release: 7 focused diagnostics/Settings tests passed, adding Main-thread installation and actual picker-return export after stopped Activity recreation to the existing persistence, caps, privacy, interval and localized UI coverage. |
| Release actual network | Final installed artifact passed in 55.943 s: real unknown-host connection failure, terminal output, externally paused Linux host, Reconnecting, resumed Active with new attachment/input epoch and unchanged Session. |
| Release process restart | Final installed artifact passed in 1.014 s after force-stop/new process: retained failure/reconnect records and remaining detail interval; explicit off applied afterward. |
| System document export | Actual Settings → Include detail → Export logs → Android Downloads → Save succeeded. Pulled file was 26,562 bytes, 87 events plus schema-1 export header, debug included, zero omitted/truncated/pending-loss counts. Every line is at most 4 KiB; terminal/command/secret sentinels are absent; failure and reconnect events are present. |
| `git diff --check` | Passed; repeated after final documentation edits. |

## Reproduction and selected fixtures

Native host gates use `just check`; focused tests are under their existing owners.
Real Iroh UDP tests are intentionally disabled on macOS by the repository policy.
The four Linux tests ran inside `colima-zterm-logging-0928` using
`rust:1.98.0-bookworm` and an independent Cargo target/cache. No macOS network
daemon or application-firewall permission prompt was needed.

Android used newly created AVD `Zterm_Logs_0928`, serial `emulator-5580`.
Release APKs were signed with the local development key for disposable runtime
verification only; this was not protected production signing or publication.
The opt-in network fixture used saved host `logging-fixture-0928` backed by
container `zterm-logging-0928-host` and independent `/root/.zterm` state. The host
coordinator paused the container only after `diagnostics-ready` and resumed it
after `diagnostics-reconnecting`, with unconditional unpause on failure.

```sh
adb -s emulator-5580 shell am instrument -w \
  -e class io.github.leonfox28.zterm.DiagnosticsNetworkTest#nativeFailureAndRealReconnectPersistWithoutTerminalContent \
  -e diagnosticsNetwork 1 -e reuseDiagnosticsHost 1 \
  io.github.leonfox28.zterm.test/androidx.test.runner.AndroidJUnitRunner
# Run with the external pause/resume coordinator; requires the selected fixture host.
adb -s emulator-5580 shell am force-stop io.github.leonfox28.zterm
adb -s emulator-5580 shell am instrument -w \
  -e class io.github.leonfox28.zterm.DiagnosticsNetworkTest#nextProcessRetainsHistoryAndOnlyRemainingDetailInterval \
  -e diagnosticsRestart 1 \
  io.github.leonfox28.zterm.test/androidx.test.runner.AndroidJUnitRunner
```

Machine-local command logs and selected evidence are retained below
`/tmp/zterm-logging-0928-*`, including `final-check.log`,
`final-android-release-build.log`, `final-android-check.log`,
`final-release-tests.log`, `network-tests.log`, `final-restart-tests.log`, `linux.log`
and `evidence/android-saf-export.jsonl`. The system picker screenshot and debug
JUnit XML are also in `evidence/`. They are disposable supporting artifacts;
the committed tests and this report provide the durable acceptance record.

## Failures investigated and regression lessons

- The Store response waiter's timeout CAS can lose to the actor's EXPIRED state,
  as well as to STARTED. Only STARTED represents an ambiguous execution outcome;
  an already-expired command must return DeadlineExceeded. A deterministic
  connected-but-empty reply fixture covers all three states and failed before
  the correction. The real persistence suite and final full gate now pass.
- Mixed RFC3339 fractional precision is not lexicographically chronological.
  Parse timestamps before merging lanes; a regression includes whole seconds,
  `.01`, `.11` and `.9` values.
- A transient control error or malformed/OFF marker must not forget the last
  valid interval's monotonic deadline. Restoring the same marker after clock
  rollback cannot renew it. Injected wall/monotonic observations test expiry,
  recovery and explicit renewal without changing the system clock.
- Queue admission originally lost ordinary events during shared mutex
  contention. Independent bounded channels/byte counters now keep worker locks
  out of ordinary key admission. Blocked-sink tests cover bounded latency and
  independent detail capacity.
- Cross-process fsync contention required bounded one-second writer-lock retries;
  retries stay on the worker. Process fixtures wait for every child before
  removing their private root.
- Parallel writer-process fixtures exposed an immediate-close/rebind assumption
  in the existing Unix listener test: it failed with `AlreadyRunning` twice,
  passed alone and passed when the spawning fixture was excluded. A concurrently
  spawned process can briefly retain the inherited listener until exec. The test
  now explicitly verifies live-socket refusal, then waits at most one second for
  retirement before asserting stale replacement. Production bind/unlink behavior
  is unchanged; the complete platform suite passes (28 plus its child helper).
- The first remote attempt hit public-relay readiness delays. A later pairing
  attempt had an unknown outcome after a long fixture interruption. Reused the
  already committed selected host, added read-only readiness, and did not replay
  ambiguous mutations or increase product network deadlines.
- A network test initially inspected eager `NativeFrame.rows`, which is empty
  when the actual content is owned by `frame.source`. Materializing
  `presentationRows()` using the existing native-test pattern allowed the real
  output assertion and subsequent reconnect checks to complete.
- CLI inventory expectations were updated for the new operations; `debug`
  actions are parsed values, not nested clap subcommands. An initially mistyped
  upload test-target command was replaced with its actual owner module.
- The first Linux link attempt exceeded the disposable VM's memory at three
  build jobs; a single-job build completed. This was fixture capacity, not a
  product failure.
- Generated Android system logcat contained unrelated OS token-like fields and
  triggered the repository secret scan. Preserved the test evidence outside the
  source tree; did not weaken the scan or change its policy.

The logging and Android specs now preserve the clock, merge, producer admission
and frame-source rules. All fixes remain within this task's logging/test scope.

## Evidence boundaries and completion

The repository's existing child-process helper ignores are exercised by parent
tests; macOS real-Iroh skips have separate Linux evidence. Hosted-only checks
(other supported hosts, glibc floor, full relay Docker/QEMU matrix, protected
signing, installers, attestation and immutable publication) remain CI-owned.
The user subsequently authorized the feature-branch push and PR creation.
Release and deployment checks remain outside this task.

Before cleanup the selected Linux host reported zero Sessions and Android's
control marker was OFF. The owned emulator and Colima VM were stopped; their
named AVD/profile/container data and the temporary pairing ticket were removed.
The pre-existing Colima profile, other AVDs and personal product state were not
part of the fixture. Export/screenshots/test logs remain under `/tmp` for review.

The user approved the concrete commit plan and PR creation on 2026-09-28.
Implementation, review and local validation are complete; the remaining work is
commit/archive/journal bookkeeping, the feature-branch push and PR creation.
