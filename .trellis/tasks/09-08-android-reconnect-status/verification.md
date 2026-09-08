# Verification — 2026-09-08

## Result and causal review

The remembered exact ID survived daemon restart while the daemon-owned Session did not. Android machine entry and Retry now resolve a successful live list before selecting; an empty result uses the existing default-main attachment contract and persists the returned identity. Explicit Session selection remains exact and occupancy still requires confirmation.

ConnectionStatus was previously discarded by the native actor. Its typed route/optional RTT now flows through NativeFrame and the distinct TerminalStatus into the 48 dp header. True attachment loss clears metrics; healthy synchronization preserves them. Metadata projection reuses the existing immutable content/source path. Direct and Relay are fixed English resources in all locales.

The actual restart fixture also found that the Iroh unary adapter passed validated ServiceError frames into a success decoder. Projecting typed errors at that adapter retires obsolete operation leases without replaying an ambiguous mutation. Regression coverage verifies both the request sequence and redaction.

## Final checks

| Check | Result |
| --- | --- |
| cargo +1.98.0 test -p zterm-android -p zterm-client --all-features | PASS: 15 Android + 73 client tests |
| cargo +1.98.0 clippy -p zterm-android -p zterm-client --all-targets --all-features -- -D warnings | PASS |
| cargo +1.98.0 fmt --all -- --check | PASS |
| sh tests/terminal-dependency-policy.sh | PASS |
| Android lintDebug, testDebugUnitTest, assembleDebug, assembleDebugAndroidTest via tools/android/build.sh | PASS; JVM unit test task has NO-SOURCE |
| Final test-fixture edit: lintDebug and assembleDebugAndroidTest | PASS |
| git diff --check | PASS |
| sh tests/source-policy.sh; sh tests/secret-scan.sh | PASS |

## Android runtime evidence

All instrumentation used the task-owned API 36 arm64 AVD Zterm_Presentation_0908 (emulator-5556) and disposable presentation-fixture daemon. The real user's daemon and Sessions were not used or restarted.

| Test | Final result | Assertions exercised |
| --- | --- | --- |
| ConnectionStatusUiTest | PASS, 1 test | 240 dp subtitle, long host name, system/Chinese/English Direct and Relay, missing RTT, unknown path, healthy sync, reconnect/end/closed/lease loss |
| ReconnectRecoveryTest with reconnectFixture=1, daemonRestart=1 | PASS, 1 test, 15.928 s | Obsolete recent ID to main; real daemon stop/start; actual Retry; new durable ID; live reuse; text input; stale mutation lease; one/multiple/occupied selection; Back fencing |
| OccupiedRecoveryTest with occupiedRecovery=1, hostName=presentation-fixture | PASS, 1 test; latest 8.544 s | Exact occupied restore, cancel/confirmed takeover, subsequent lease loss and second explicit takeover |
| AttachGeometryTest with attachGeometry=1, hostName=presentation-fixture | PASS, 3 tests, 5.319 s | Measurements during unfinished attachment reach the host's real grid, IME completion and integral geometry |
| TerminalRenderingTest | PASS, 6 tests | Cached/unknown rows, edge waits, output arrival and actual hardware-rendered pixel preservation |

Total: 12 passing Android instrumentation tests. The app accepted printf input after recovery and displayed RECOVERY_OK in the actual terminal. The screenshot /tmp/zterm-reconnect-header.png was visually inspected: main above presentation-fixture · Direct · 6 ms, with the existing header/grid/shortcuts intact.

Initial geometry runs failed at the first read-only cold-endpoint query before creating or attaching any Session. The fixture now bounds read-only readiness attempts to 20 seconds; it does not retry Session creation or any assertion under test. No product deadline was extended. Another initial restart run correctly reported a failed list while the fixture was merely bound, before its network became online; the coordinator now waits for online before releasing Retry.

## Coverage limits

Actual runtime routing was Direct. Forced Relay routing, real idle Direct/Relay migration and a physical Android phone were not induced/tested. Both route values and state transitions were exercised at the shared typed projection/Compose layers. The one-second idle observer was reviewed against the installed noq RecvStream's cancel-safe read contract: it preserves decoder state, emits only changed samples and writes no terminal input. This is not claimed as a real network migration test.

Unauthorized/error redaction and ambiguous mutation sequencing have shared adapter regression coverage; occupancy and ambiguity additionally ran against the fixture. Navigation tests cover Back during suspended connection work. No forced authorization revocation or exhaustive storage/navigation race injection was performed.

## Artifacts

- App: apps/android/app/build/outputs/apk/debug/app-debug.apk
- SHA-256: 27d9f195b82cb73ea45448eccb85bcb5ba96377db21ac58b6c75a2d9af481d9c
- Test APK: apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
- SHA-256: feb9a6622b52d308765a815d2478577c8c9df210df2dce0ac2b71a560096737b

Generated native/Kotlin outputs came from the normal Android build and were not manually edited. No new protocol, storage schema, dependency or release publication was required.

## Final review and cleanup

The main session completed trellis-check across Android frontend, native bridge and shared client, including selection/error data flow, authoritative identity persistence, source ownership, generated binding compatibility and spec synchronization. No sub-agents were used. The fixture had zero running Sessions at cleanup; its daemon was gracefully stopped and the task-owned emulator shut down. Work is ready for the single Phase 3.4 commit confirmation.
