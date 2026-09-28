# Proposed commit plan

Branch: `feat/logging-improvements`. Base: `main` at `acecb45`.
Status: ready after the second review and approved Store correction; final
`just check` passed. The user explicitly approved commits and PR creation on
2026-09-28; this plan is now being executed.

## 1. Work commit

`feat: add bounded local logs and on-demand diagnostics`

One coherent compatibility change: migrate all desktop writers together, share
one safe event/control contract with Android, update inspection/export and keep
managed cleanup, documentation and owner-level regressions in the same commit.
Includes the user-approved Store timeout classification correction discovered by
the full quality gate, with its deterministic regression and contract update.

Validation and evidence: [verification.md](verification.md), including the
user-requested second review and corrections in [review.md](review.md).
Implemented user-facing behavior: [logging guide](../../../docs/logging.md).

Exact file list (79 paths):

- `.trellis/spec/backend/effective-user-state.md`
- `.trellis/spec/backend/index.md`
- `.trellis/spec/backend/local-daemon-ipc.md`
- `.trellis/spec/backend/logging-guidelines.md`
- `.trellis/spec/backend/shared-client.md`
- `.trellis/spec/frontend/android-app.md`
- `.trellis/tasks/09-26-logging-improvements/check.jsonl`
- `.trellis/tasks/09-26-logging-improvements/commit-plan.md`
- `.trellis/tasks/09-26-logging-improvements/design.md`
- `.trellis/tasks/09-26-logging-improvements/implement.jsonl`
- `.trellis/tasks/09-26-logging-improvements/implement.md`
- `.trellis/tasks/09-26-logging-improvements/prd.md`
- `.trellis/tasks/09-26-logging-improvements/research/logging-audit.md`
- `.trellis/tasks/09-26-logging-improvements/review.md`
- `.trellis/tasks/09-26-logging-improvements/task.json`
- `.trellis/tasks/09-26-logging-improvements/verification.md`
- `Cargo.lock`
- `Cargo.toml`
- `README.md`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/DiagnosticsNetworkTest.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/DiagnosticsTest.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/SettingsUiTest.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppDiagnostics.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppUi.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppUpdates.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/DiagnosticsSettings.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/MainActivity.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalNotifications.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/ZtermApplication.kt`
- `apps/android/app/src/main/res/values-zh/strings.xml`
- `apps/android/app/src/main/res/values/strings.xml`
- `crates/android/Cargo.toml`
- `crates/android/src/diagnostics.rs`
- `crates/android/src/lib.rs`
- `crates/android/src/terminal.rs`
- `crates/cli/Cargo.toml`
- `crates/cli/src/lib.rs`
- `crates/cli/src/main.rs`
- `crates/cli/src/terminal_ui.rs`
- `crates/cli/tests/command_side_effects.rs`
- `crates/cli/tests/daemon_autospawn.rs`
- `crates/client/Cargo.toml`
- `crates/client/src/session.rs`
- `crates/client/src/upload.rs`
- `crates/client/src/view.rs`
- `crates/core/src/domain.rs`
- `crates/daemon/Cargo.toml`
- `crates/daemon/src/connection_broker.rs`
- `crates/daemon/src/connection_progress.rs`
- `crates/daemon/src/diagnostics.rs`
- `crates/daemon/src/lib.rs`
- `crates/daemon/src/lifecycle.rs`
- `crates/daemon/src/local_ipc.rs`
- `crates/daemon/src/operations.rs`
- `crates/daemon/src/service.rs`
- `crates/daemon/src/session.rs`
- `crates/daemon/src/session_wire.rs`
- `crates/daemon/src/session_wire/upload.rs`
- `crates/daemon/src/store.rs`
- `crates/daemon/src/update.rs`
- `crates/daemon/src/update/tests.rs`
- `crates/daemon/tests/local_device_ipc.rs`
- `crates/daemon/tests/support/daemon_harness.rs`
- `crates/diagnostics/Cargo.toml`
- `crates/diagnostics/src/event.rs`
- `crates/diagnostics/src/lib.rs`
- `crates/diagnostics/src/reader.rs`
- `crates/diagnostics/src/recorder.rs`
- `crates/diagnostics/src/tests.rs`
- `crates/platform/Cargo.toml`
- `crates/platform/src/diagnostics.rs`
- `crates/platform/src/lib.rs`
- `crates/platform/src/local_unix.rs`
- `crates/platform/src/user_state.rs`
- `docs/android.md`
- `docs/core-local-daemon.md`
- `docs/logging.md`
- `docs/remote-cli.md`

## Unrecognized dirty files

None. All listed paths belong to this task's implementation, checks, specs or
planning/verification artifacts. This inventory was verified before staging.

## Follow-up bookkeeping

After the work commit, use the project's finish-work scripts to archive
`09-26-logging-improvements` and record its session journal. Expected order:

1. The work commit above.
2. `chore(task): archive 09-26-logging-improvements`.
3. `chore: record journal`.

The user approved this commit plan, task completion bookkeeping, pushing
`feat/logging-improvements` and creating a PR to `main` on 2026-09-28.
