# Proposed commit

Status: the user authorized this commit batch, PR creation and merge on
2026-09-17 with “提交然后pr、合并吧”.
Branch: `codex/terminal-notifications`.

1. `feat(terminal): forward live notifications to desktop and Android`

One coherent change includes the OSC parser/domain, kind 325, controller queues,
desktop presenter, native/Kotlin Android delivery, tests, documentation and task
artifacts. Keeping producers and receivers in one commit makes the required
matching-build contract reviewable and reversible as a unit.

## Exact file list

- `.trellis/spec/backend/core-wire-domain.md`
- `.trellis/spec/backend/index.md`
- `.trellis/spec/backend/local-daemon-ipc.md`
- `.trellis/spec/backend/shared-client.md`
- `.trellis/spec/backend/terminal-driver.md`
- `.trellis/spec/backend/terminal-model.md`
- `.trellis/spec/backend/terminal-notifications.md`
- `.trellis/spec/frontend/android-app.md`
- `.trellis/tasks/09-17-terminal-notifications/check.jsonl`
- `.trellis/tasks/09-17-terminal-notifications/commit-plan.md`
- `.trellis/tasks/09-17-terminal-notifications/design.md`
- `.trellis/tasks/09-17-terminal-notifications/implement.jsonl`
- `.trellis/tasks/09-17-terminal-notifications/implement.md`
- `.trellis/tasks/09-17-terminal-notifications/prd.md`
- `.trellis/tasks/09-17-terminal-notifications/research/android-notifications.md`
- `.trellis/tasks/09-17-terminal-notifications/research/herdr-notifications.md`
- `.trellis/tasks/09-17-terminal-notifications/research/notification-path.md`
- `.trellis/tasks/09-17-terminal-notifications/research/notification_acceptance.rs`
- `.trellis/tasks/09-17-terminal-notifications/task.json`
- `.trellis/tasks/09-17-terminal-notifications/validation.md`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/TerminalNotificationsTest.kt`
- `apps/android/app/src/main/AndroidManifest.xml`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppUi.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalNotifications.kt`
- `apps/android/app/src/main/res/drawable/ic_notification.xml`
- `apps/android/app/src/main/res/values-zh/strings.xml`
- `apps/android/app/src/main/res/values/strings.xml`
- `crates/android/src/terminal.rs`
- `crates/android/src/terminal/navigation.rs`
- `crates/android/src/terminal/notifications.rs`
- `crates/cli/src/terminal_ui.rs`
- `crates/cli/src/terminal_ui/ansi_presenter.rs`
- `crates/cli/src/terminal_ui/session.rs`
- `crates/cli/tests/daemon_autospawn.rs`
- `crates/client/src/iroh_controller.rs`
- `crates/client/src/session.rs`
- `crates/client/src/view.rs`
- `crates/core/src/terminal.rs`
- `crates/core/src/terminal/notifications.rs`
- `crates/daemon/src/session.rs`
- `crates/daemon/src/session_wire.rs`
- `crates/daemon/src/terminal_driver.rs`
- `crates/daemon/tests/local_session_ipc.rs`
- `crates/daemon/tests/support/daemon_harness.rs`
- `crates/proto/build.rs`
- `crates/proto/src/lib.rs`
- `crates/terminal/src/ingress.rs`
- `crates/terminal/src/lib.rs`
- `crates/terminal/src/model.rs`
- `crates/terminal/tests/notifications.rs`
- `crates/terminal/tests/security_policy.rs`
- `crates/terminal/tests/terminal_corpus.rs`
- `docs/android.md`
- `docs/persistent-sessions.md`
- `proto/zterm/v2/terminal.proto`
- `proto/zterm/v2/wire.proto`

## Unrecognized dirty files

None. The branch started from a clean tree; every listed path was edited or
created for this task. Generated Android/native build outputs are ignored and
are not included. Archive/journal bookkeeping follows the feature commit as a
separate workflow step; no other active task is included.

## Validation

See [validation.md](validation.md). The complete repository gate and Android API
36/API 32 acceptance passed. The user also confirmed both Ghostty OSC 9/777
notifications display normally, closing the remaining manual acceptance item.
