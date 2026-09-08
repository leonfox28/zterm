# Proposed commits (in order)

1. `fix(android): recover stale sessions and show connection status`

   - `.trellis/spec/backend/shared-client.md`
   - `.trellis/spec/frontend/android-app.md`
   - `.trellis/tasks/09-08-android-reconnect-status/check.jsonl`
   - `.trellis/tasks/09-08-android-reconnect-status/commit-plan.md`
   - `.trellis/tasks/09-08-android-reconnect-status/design.md`
   - `.trellis/tasks/09-08-android-reconnect-status/implement.jsonl`
   - `.trellis/tasks/09-08-android-reconnect-status/implement.md`
   - `.trellis/tasks/09-08-android-reconnect-status/prd.md`
   - `.trellis/tasks/09-08-android-reconnect-status/task.json`
   - `.trellis/tasks/09-08-android-reconnect-status/verification.md`
   - `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/AttachGeometryTest.kt`
   - `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/ConnectionStatusUiTest.kt`
   - `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/OccupiedRecoveryTest.kt`
   - `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/ReconnectRecoveryTest.kt`
   - `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/TerminalRenderingTest.kt`
   - `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/TerminalUiTest.kt`
   - `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt`
   - `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalScreen.kt`
   - `apps/android/app/src/main/res/values-zh/strings.xml`
   - `apps/android/app/src/main/res/values/strings.xml`
   - `crates/android/src/terminal.rs`
   - `crates/android/src/terminal/navigation.rs`
   - `crates/client/src/iroh_controller.rs`
   - `docs/android.md`

Unrecognized dirty files: none. Every listed file belongs to this task.

After this work commit, run Trellis finish-work to archive only this task and record its journal (separate script-generated bookkeeping commits). No push or release publication.

Status: approved on 2026-09-08 when the user requested the release workflow. The work commit and Trellis bookkeeping precede release preparation; publication is separately authorized by that request.
