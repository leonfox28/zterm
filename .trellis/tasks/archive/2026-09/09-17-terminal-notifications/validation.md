# Notification feature validation

Date: 2026-09-17. Branch: `codex/terminal-notifications`, base `71f41d3`.
The user authorized implementation with “开始执行吧” and, after acceptance,
authorized the commit, PR and merge with “提交然后pr、合并吧”.

## Repository checks

- `just check` passed on macOS arm64 with Rust 1.98.0. Rust test summaries report
  **681 passed, 0 failed, 8 ignored**. Custom executable PTY/CLI harnesses also
  completed successfully. The gate includes formatting, source/release/secret
  policies, Clippy, workspace tests, docs, dependency audits and relay static checks.
- `just check-fast` passed again after the final clipboard fixture wording cleanup.
- `sh tools/android/build.sh :app:assembleDebug :app:assembleDebugAndroidTest
  :app:lintDebug :app:testDebugUnitTest` passed. Later Android-test-only changes
  were rebuilt with `:app:assembleDebugAndroidTest :app:lintDebug` and rerun.
- `git diff --check` passed. Kotlin bindings came from the existing UniFFI build,
  with no generated-source edits or new dependency.
- Existing host restrictions remain: the eight ignored tests include real Iroh
  UDP-listener cases on macOS. Other supported host architectures and hosted CI
  distribution/signing/installer gates were not represented as local evidence.

Full gate log: `/tmp/zterm-notifications-check.log`; final fast gate:
`/tmp/zterm-notifications-final-fast.log`; APK checks:
`/tmp/zterm-notifications-android-build.log` and
`/tmp/zterm-notifications-android-test-build.log`.

## Acceptance mapping

| PRD | Evidence |
| --- | --- |
| A1 | Actual daemon PTY → local IPC → CLI outer PTY delivers both canonical forms once. Shared client tests validate remote/local route parity; the real Android remote test receives both forms through Iroh. Presenter formatting is route-independent. |
| A2 | `terminal/tests/notifications.rs` tests every chunk width, BEL/ESC-ST/C1 framing, Chinese text, and the C1-shaped continuation byte in `果`. |
| A3 | Ingress alternate-screen/DEC 2026 test preserves screen, cursor, title handling and independent clipboard effects. Presenter tests preserve visual baseline and assert one flush. Existing clipboard/security/corpus suites pass. |
| A4 | Broker/shared-client/native tests clear retired queues. Real Android detach/reconnect test emits `missed` while detached, observes no delivery or replay, then receives only `fresh`. |
| A5 | Broker target transition/observer tests and existing Session current-generation eligibility, takeover and terminal-lifecycle tests pass. Native generation checks invalidate already-consumed old events. |
| A6 | Core/broker/shared/native tests cover FIFO order, identical repeated events, cap 32, oldest eviction and coalesced watches. Writer and reader alternate effects with ordinary visual work while lifecycle transitions retain priority. |
| A7 | Domain/proto/ingress tests reject progress/subcommands, malformed/oversized/control text and cancellation; nested Debug is content-redacted. Wire tests reject nonzero correlation/deadline and wrong attachment. |
| A8 | Herdr-format `结果: done` survives actual CLI forwarding. The user completed the supplied OSC 9/777 Ghostty popup smoke and reported “试过了，两个都正常”; see the manual evidence below. |
| A9 | API 36 real remote attachment posts OSC 9 and OSC 777 with Unicode in foreground and while Activity is CREATED/backgrounded with the connection active. System field tests assert title/body/channel/private visibility/content intent. |
| A10 | API 36 Activity recreation plus an output barrier produces no extra content notification. Native tests cover waiter cancellation, closure and generation invalidation across reconnect. Real detach/reconnect test passes. |
| A11 | API 36 tests pass for grant, denial followed by grant without replay, and disabled-channel policy preservation. API 32 ordinary posting and disabled-channel tests also pass without a notification runtime grant. |

## Android runtime evidence

The disposable `Zterm_Notifications_0917` emulator used Android API 36,
`google_apis;arm64-v8a`, serial `emulator-5566`. Only this owned debug install was
cleared or granted/revoked notification permission. Four focused cases passed:

1. `grantedPermissionPostsDistinctTypedNotifications` — separate repeated
   notifications and exact Unicode/title/body mapping.
2. `deniedPermissionConsumesWithoutPosting` — a denied event stays skipped after
   runtime permission is granted; a new event can then post.
3. `disabledChannelConsumesWithoutOverridingUserPolicy` — an explicitly disabled
   channel stays disabled and posts nothing.
4. `liveConnectionPostsInBackgroundWithoutRecreationOrReconnectReplay` — actual
   host PTY, typed Iroh transport, shared client, native event bridge, repository
   collector and system notification API. Covers foreground/background,
   recreation, detach, missed output and fresh output after reconnect.

The live case passed in 25.105 seconds. It used the disposable real-network host
helper [notification_acceptance.rs](research/notification_acceptance.rs), one
owned session and a task-private working directory. Pairing tickets were passed
only through private files, never printed. Repeated runs reused the legitimately
paired host/identity; no authorization or pairing logic was bypassed or changed.
Initial pairing/discovery attempts hit `pair_outcome_unknown`/deadline failures;
the fixture bounds read-only readiness retries and reports failures by phase.

Instrumentation log files use `/tmp/zterm-notifications-android-` with suffixes
`granted.log`, `denied.log`, `channel.log`, and `live.log`. AndroidJUnitRunner's
final instrumentation code alone does not determine success; each recorded
passing run contains `OK (1 test)` and no failed/ignored test status.

Two fixture corrections prevented false conclusions:

- Android API 36 adds an automatic group summary asynchronously. Three content
  notifications produced four active records, one with `FLAG_GROUP_SUMMARY`
  (flags 1808) and no body. Tests now count content notifications separately.
  This was not an Activity recreation replay. The platform describes
  [automatic grouping](https://developer.android.com/develop/ui/views/notifications/group#automatic-grouping).
- A null repository frame precedes completion of asynchronous host detach.
  The test now waits until the host reports the session unoccupied, rather than
  mistaking local UI retirement for completed remote detach. An execution marker
  arms delayed output before detach and verifies its completion before `fresh`.

The disposable `Zterm_Notifications_API32_0917` emulator used Android API 32,
`google_apis;arm64-v8a`, serial `emulator-5568`. Ordinary posting with distinct
Unicode notifications and disabled-channel suppression both passed, without
any POST_NOTIFICATIONS runtime grant. Logs:
`/tmp/zterm-notifications-api32-{granted,channel}.log` and
`/tmp/zterm-notifications-api32-tests.log`.

Both owned emulators were stopped and their task-created AVDs deleted. The
private test host was stopped and its credentials/session working directory
removed. No user's existing emulator or production daemon was changed.

## Ghostty / Herdr boundary

Ghostty 1.3.1's installed help/config confirms desktop OSC notification support.
A temporary isolated Ghostty process was launched with synthetic OSC 9/777 and
Chinese text. The UI tool then rejected access to `com.mitchellh.ghostty` for
safety reasons. No alternate UI-control path was used to bypass that restriction.
The test process exited automatically. This tool run established no popup proof;
the later user verification below supplies that evidence.

The user was given the following one-line manual smoke, with instructions to
switch focus to another app within three seconds:

```sh
sleep 3; printf '\033]9;OSC 9 测试：中文结果正常\007'; sleep 3; printf '\033]777;notify;OSC 777 测试;中文正文正常\033\\'
```

The user then reported: “试过了，两个都正常” (both worked normally).
Record this as user-reported actual notification display for both forms,
complementing the automated zterm forwarding evidence; it was not observed by
the UI tool. This closes A8's outstanding Ghostty popup check.

Herdr 0.9.1's upstream Ghostty backend and local terminal-delivery configuration
were inspected during research. Deterministic Herdr-format output was tested;
no claim is made that all hosted/remote environments make Herdr select that
backend. Zterm's hosted TERM and application detection remain unchanged.

## Final review

The feature stays within the approved OSC 9/777 ordinary-notification scope.
There is no persistent notification state, push, foreground service, wake lock,
new background connection owner, protocol fallback or application-specific path.
Wire kind 325 requires matching updated host/client builds. Documentation and
Trellis contracts cover that rollout boundary and the transient queue/lifetime
rules. No unrelated working-tree changes were found.
