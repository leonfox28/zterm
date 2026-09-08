# Commit records and pending follow-up

## Completed v0.1.27 release commits

Authorized by the user on 2026-09-08: “先走发布流程吧”.
The canonical release workflow may commit, push, merge and publish v0.1.27.
No amend or running-daemon update. All listed paths were edited for
this task; no unrecognized dirty paths were found. Work remains on
`feat/terminal-presentation-continuity`.

1. `feat(terminal): publish synchronized output at coherent boundaries`

   - `.trellis/spec/backend/session-service.md`
   - `.trellis/spec/backend/terminal-driver.md`
   - `.trellis/spec/backend/terminal-model.md`
   - `crates/daemon/src/session.rs`
   - `crates/daemon/src/session_wire.rs`
   - `crates/daemon/src/terminal_driver.rs`
   - `crates/terminal/src/ingress.rs`
   - `crates/terminal/src/model.rs`
   - `crates/terminal/tests/security_policy.rs`
   - `crates/terminal/tests/synchronized_output.rs`

2. `feat(client): preserve presentation and input across resize`

   - `.trellis/spec/backend/local-daemon-ipc.md`
   - `.trellis/spec/backend/shared-client.md`
   - `.trellis/spec/frontend/android-app.md`
   - `.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/task.json`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/check.jsonl`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/commit-plan.md`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/design.md`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/implement.jsonl`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/implement.md`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/prd.md`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/progress.md`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/research/cross-client-presentation.md`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/research/runtime-acceptance.md`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/research/release-0.1.27.md`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/research/second-review.md`
   - `.trellis/tasks/09-07-terminal-presentation-continuity/task.json`
   - `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/AttachGeometryTest.kt`
   - `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/NativeTerminalTest.kt`
   - `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/TerminalUiTest.kt`
   - `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt`
   - `apps/android/app/src/main/java/io/github/leonfox28/zterm/ImeAnimationState.kt`
   - `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalGeometry.kt`
   - `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalScreen.kt`
   - `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt`
   - `crates/android/src/terminal.rs`
   - `crates/android/src/terminal/navigation.rs`
   - `crates/cli/examples/presentation_fixture.rs`
   - `crates/cli/src/terminal_ui.rs`
   - `crates/cli/src/terminal_ui/ansi_presenter.rs`
   - `crates/cli/src/terminal_ui/composition.rs`
   - `crates/cli/src/terminal_ui/session.rs`
   - `crates/cli/src/terminal_ui/session_tests.rs`

Validation: full `just check`, final Android build/lint, focused host/native/desktop
tests, and Android real-IME/pixel/gesture tests passed. See
[acceptance evidence](research/runtime-acceptance.md) for exact coverage and limits.
Desktop GUI visual acceptance is still unavailable; approval to commit does not
turn that unperformed check into a pass. Archive/journal bookkeeping follows the
project finish workflow once completion is accepted.

The user authorized release before the remaining flicker work. The Android
keyboard-close blank/history-refill issue remains open, along with full-transition
visual acceptance. Keep this task active after publication. The canonical operator
adds a separate two-file `chore: prepare v0.1.27 release` commit after these groups.

## Authorized Android scrolling release — 2026-09-08

The user approved optimization after the profiling discussion. The prior
uncommitted ghosting correction is now included in the coherent row-window
implementation; the earlier proposal is superseded by this file grouping.
After testing the updated phone, the user confirmed “测试了一下非常棒，现在提交所有变更，然后走发布流程吧”. This authorizes the complete work commit, version preparation, push/PR/merge, normal protected signing approval and publication as v0.1.28. All paths below are task-owned; no unrecognized dirty files.

`perf(android): scroll cached terminal rows locally`

- `crates/android/src/terminal.rs`
- `crates/android/src/terminal/navigation.rs`
- `crates/core/src/viewport_cache.rs`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalGeometry.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalScreen.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalRowRenderer.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/NativeTerminalTest.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/TerminalUiTest.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/TerminalRenderingTest.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/TerminalScrollProfileTest.kt`
- `.trellis/spec/backend/shared-client.md`
- `.trellis/spec/frontend/android-app.md`
- `.trellis/tasks/09-07-terminal-presentation-continuity/commit-plan.md`
- `.trellis/tasks/09-07-terminal-presentation-continuity/design.md`
- `.trellis/tasks/09-07-terminal-presentation-continuity/implement.md`
- `.trellis/tasks/09-07-terminal-presentation-continuity/prd.md`
- `.trellis/tasks/09-07-terminal-presentation-continuity/progress.md`
- `.trellis/tasks/09-07-terminal-presentation-continuity/task.json`
- `.trellis/tasks/09-07-terminal-presentation-continuity/research/android-scroll-handoff.md`
- `.trellis/tasks/09-07-terminal-presentation-continuity/research/android-scroll-optimization.md`

Validation and remaining visual boundaries are in
[scrolling acceptance](research/android-scroll-optimization.md). Keep the task
active for prior keyboard-close/history-refill and broader desktop visual checks. Phone scrolling acceptance is now confirmed.
