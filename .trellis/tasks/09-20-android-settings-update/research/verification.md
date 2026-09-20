# Implementation verification — 2026-09-20

Branch: `feat/android-settings-update`, created from `main` before product edits.
The user's `好 开始执行吧，另外你没新创建branch` authorized implementation.
Code/spec/verification work is complete. The user confirmed the Phase 3.4 local
commit and task archival with `确认提交并归档`. [Approved commit](commit-plan.md).

## Delivered behavior

- Settings footer: fixed repository browser intent, download icon without a
  navigation chevron, current version and manual pending state.
- Manual current-version result uses Android's native text Toast. Automatic
  current/error results have no visible effect. One cold-process gate and
  manual promotion prevent duplicate requests; active terminal/scanner or modal
  work defers unsolicited offers until an eligible foreground screen.
- Later saves a same-version 24-hour reminder preference. Manual checks and
  higher versions bypass it. No scheduler, automatic download or forced install.
- Shared Rust release authentication binds both detached signatures, checksum
  inventory, metadata and APK. Kotlin bounds HTTP streams, checks the APK identity
  and delegates installation through a narrow FileProvider URI and OS permission.
- App notification switch persists independently of OS permission, drops events
  while Off, and rolls back to the committed gate on storage failure.
- Failure/retry use the same responsive wide card with full-size actions,
  preserving existing session/retry/takeover state semantics.

## Checks and results

| Check | Result |
| --- | --- |
| `cargo fmt --package zterm-core --package zterm-android --package zterm-release-tool` | Clean final formatting |
| `cargo clippy -p zterm-core -p zterm-android -p zterm-release-tool --all-targets -- -D warnings` | Passed |
| `cargo test -p zterm-core -p zterm-android -p zterm-release-tool` | 114 passed: core74, pairing10, Android21, release-tool9 |
| `sh tools/android/build.sh :app:testDebugUnitTest :app:lintDebug :app:lintRelease :app:assembleDebug` | Passed; 9 JVM tests, both lint variants pass (nonfatal repository warnings remain) |
| `sh tools/android/build.sh :app:lintDebug :app:assembleDebugAndroidTest` | Passed after final test edits |
| `python3 tools/android/check-apk.py --sdk "$ANDROID_HOME" apps/android/app/build/outputs/apk/debug/app-debug.apk` | ZIP and every packaged arm64 ELF 16 KB alignment passed |
| `git diff --check` | Passed |

The Gradle commands include the `just android-check` / `just android-build`
tasks. Native bindings were regenerated exclusively through `:app:buildRust`.
No generated Kotlin was hand-edited. No dependency or product version was bumped.

## Android execution

Task-owned headless AVD `Zterm_Settings_0920`, API36 Google APIs arm64, serial
`emulator-5554`; no personal device or existing host/session was used. Run with
`adb -s emulator-5554 shell am instrument -w -e class <selectors>` and runner
`io.github.leonfox28.zterm.dev.test/androidx.test.runner.AndroidJUnitRunner`.

Final main selection: **14 tests passed**, with no assumption-gated tests included:

- SettingsUiTest: footer language/theme matrix, notification persistence/GitHub
  intent, all font slider steps and Activity recreation.
- IdentityStoreTest: all four tests including old-state migration, Off/reminder
  restart persistence, invalid reminder isolation and protected identity cases.
- UpdateUiTest: all four tests, including an accessibility event from
  `android.widget.Toast` exactly once, visible pending state, deferred automatic
  offer, zero downloads on Later, FileProvider confinement and equal failure/retry
  geometry. Compact320dp/short300dp and 1.6x font layouts remain scrollable.
- ConnectionStatusUiTest: route/state/latency regression.
- TerminalNotificationsTest: distinct system notifications and app Off/no replay.

Four separate explicit fixture selections also passed (**18 Android tests total**):

1. `SettingsUiTest#notificationPermissionDenialStaysOffAndLaterGrantEnablesDelivery`
   with `notificationPermissionFixture=1`; start with POST_NOTIFICATIONS revoked
   and user-set/user-fixed flags cleared. Operates the real OS deny/allow buttons.
2. `UpdateInstallTest` with `installerFixture=1`; start with install appops denied.
   Uses a copy of this disposable app's matching-signature installed APK. Opens OS
   settings, returns denied, grants source permission, reaches the real system
   **Update/Cancel** confirmation, then cancels and observes Idle. Restore appops
   from the host after instrumentation, since revocation kills the target process.
3. `TerminalNotificationsTest#disabledChannelConsumesWithoutOverridingUserPolicy`
   with `blockedChannel=1`; disabled channel remains disabled and posts nothing.
4. `SettingsUiTest#failedNotificationSaveRestoresCommittedGate` with
   `notificationSaveFailureFixture=1`; a test-owned AtomicFile blocker proves
   immediate Off, failed-save rollback, persisted state preservation and cleanup.

## Visual review

Chinese/English and dark/light settings captures were inspected. Footer rows,
switch and typography match the approved hierarchy; the update row has no chevron.
Representative captures:

- [Chinese dark settings](verification/settings-zh-dark.png)
- [English light settings](verification/settings-en-light.png)
- [Actual Android installer confirmation](verification/installer.png)

The installer capture is a test fixture, not a claim that version 0.1.34 is newer.

## Review findings resolved

- Cleanup after a dispatcher-changing NonCancellable block can be skipped when
  returning into a canceled coroutine. Partial deletion and native handle closure
  now both occur inside cleanup; immediate and in-flight cancellation tests pass.
- Keep candidate ownership through the IO dispatcher return to avoid dropping a
  newly verified native handle during prompt cancellation.
- Separate exact publisher SDK policy from runtime compatibility: future target
  SDKs must not strand installed updaters. Runtime validates metadata relationships
  and device minimum; release tooling still requires this tree's 26/36 policy.
- Notification permission button text uses a typographic apostrophe on API36;
  fixture tests use system resource IDs. Installer screenshots wait for its actual
  enabled confirmation action and a quiet frame, not merely its process name.
- Obsolete reminder records are ignored/replaced, instead of adding a startup
  write solely to delete one bounded harmless record.

## Evidence limits

No official release was published or installed. A genuine production upgrade
requires an official installed build and a newer protected signed release.
Development builds intentionally reject official in-place updates before network.
API26 and physical-device behavior were not executed in this session; minimum-API
compilation/lint and API36 runtime evidence must not be described as phone evidence.
No live terminal transport implementation changed; its existing native regression
tests and the connection-state/geometry UI tests passed.

Debug APK SHA-256:
`2d08981a12f10484fb99f1131a2def24cb50b7ca7dd5c60c4b1647ea89f8654d`.
Local artifact: `apps/android/app/build/outputs/apk/debug/app-debug.apk`.
