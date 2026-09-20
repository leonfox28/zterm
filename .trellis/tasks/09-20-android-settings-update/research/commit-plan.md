# Approved local commit

The user confirmed this commit and task archival with `确认提交并归档`.
Approval applies to the following file set and message. No push is planned.

`feat(android): add update checks and unified settings`

Includes the authenticated Android update path, startup/manual policy, system Toast,
notification switch, wide connection status card, tests, specs and task evidence.

## Included paths

- `.trellis/spec/backend/distribution-lifecycle.md`
- `.trellis/spec/backend/terminal-notifications.md`
- `.trellis/spec/frontend/android-app.md`
- `apps/android/app/build.gradle.kts`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/IdentityStoreTest.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/SettingsUiTest.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/TerminalNotificationsTest.kt`
- `apps/android/app/src/main/AndroidManifest.xml`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppStore.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppUi.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/MainActivity.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalNotifications.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalScreen.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/ZtermApplication.kt`
- `apps/android/app/src/main/res/values-zh/strings.xml`
- `apps/android/app/src/main/res/values/strings.xml`
- `crates/android/src/lib.rs`
- `crates/core/src/release.rs`
- `tools/release/src/assets.rs`
- `.trellis/tasks/09-20-android-settings-update/`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/UpdateInstallTest.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/UpdateUiTest.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AndroidUpdateSource.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppUpdates.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/UpdateUi.kt`
- `apps/android/app/src/main/res/drawable/ic_github.xml`
- `apps/android/app/src/main/res/xml/update_paths.xml`
- `apps/android/app/src/test/`
- `crates/android/src/updates.rs`
- `crates/core/src/release/`

## Existing work excluded

- `.codex/PENPOT.md`
- `.codex/config.toml`
- `.codex/penpot-mcp.py`
- `.gitignore`
- `.trellis/tasks/09-06-android-app/research/ui-design.md`

The task directory includes this reviewable plan. Archive/journal steps follow
only after the work commit succeeds; they remain separate commits.
