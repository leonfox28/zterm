# Proposed work commit

`feat: add remote file uploads for desktop and Android`

One coherent feature commit: domain/wire/storage/shared client, desktop and Android
frontends, focused tests, docs/spec and the four task artifacts.
The user accepted the final UI and authorized the release flow on 2026-09-09.
After this work commit and Trellis bookkeeping, prepare v0.1.31 in the same PR,
then use the canonical protected merge/sign/publish operator.
All paths below were created or edited by this session; no unrecognized dirty files.

- `.trellis/spec/backend/core-wire-domain.md`
- `.trellis/spec/backend/effective-user-state.md`
- `.trellis/spec/backend/file-upload.md`
- `.trellis/spec/backend/index.md`
- `.trellis/spec/backend/local-daemon-ipc.md`
- `.trellis/spec/backend/shared-client.md`
- `.trellis/spec/backend/terminal-input-commands.md`
- `.trellis/spec/frontend/android-app.md`
- `.trellis/tasks/09-09-android-file-upload/check.jsonl`
- `.trellis/tasks/09-09-android-file-upload/design.md`
- `.trellis/tasks/09-09-android-file-upload/implement.jsonl`
- `.trellis/tasks/09-09-android-file-upload/implement.md`
- `.trellis/tasks/09-09-android-file-upload/prd.md`
- `.trellis/tasks/09-09-android-file-upload/research/two-buttons-and-picker-back.md`
- `.trellis/tasks/09-09-android-file-upload/research/upload_acceptance.rs`
- `.trellis/tasks/09-09-android-file-upload/research/validation.md`
- `.trellis/tasks/09-09-android-file-upload/task.json`
- `.trellis/tasks/09-09-desktop-file-upload/check.jsonl`
- `.trellis/tasks/09-09-desktop-file-upload/design.md`
- `.trellis/tasks/09-09-desktop-file-upload/implement.jsonl`
- `.trellis/tasks/09-09-desktop-file-upload/implement.md`
- `.trellis/tasks/09-09-desktop-file-upload/prd.md`
- `.trellis/tasks/09-09-desktop-file-upload/research/validation.md`
- `.trellis/tasks/09-09-desktop-file-upload/task.json`
- `.trellis/tasks/09-09-file-upload-service/check.jsonl`
- `.trellis/tasks/09-09-file-upload-service/design.md`
- `.trellis/tasks/09-09-file-upload-service/implement.jsonl`
- `.trellis/tasks/09-09-file-upload-service/implement.md`
- `.trellis/tasks/09-09-file-upload-service/prd.md`
- `.trellis/tasks/09-09-file-upload-service/research/implementation.md`
- `.trellis/tasks/09-09-file-upload-service/research/validation.md`
- `.trellis/tasks/09-09-file-upload-service/task.json`
- `.trellis/tasks/09-09-remote-file-upload/check.jsonl`
- `.trellis/tasks/09-09-remote-file-upload/design.md`
- `.trellis/tasks/09-09-remote-file-upload/implement.jsonl`
- `.trellis/tasks/09-09-remote-file-upload/implement.md`
- `.trellis/tasks/09-09-remote-file-upload/prd.md`
- `.trellis/tasks/09-09-remote-file-upload/research/commit-plan.md`
- `.trellis/tasks/09-09-remote-file-upload/research/file-upload-scope.md`
- `.trellis/tasks/09-09-remote-file-upload/research/initial-assessment.md`
- `.trellis/tasks/09-09-remote-file-upload/research/remote-clipboard.md`
- `.trellis/tasks/09-09-remote-file-upload/research/validation.md`
- `.trellis/tasks/09-09-remote-file-upload/task.json`
- `Cargo.lock`
- `Cargo.toml`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/NativeUploadTest.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/TerminalUiTest.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/UploadPickerUiTest.kt`
- `apps/android/app/src/androidTest/java/io/github/leonfox28/zterm/UploadUiTest.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppUi.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalScreen.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalUploadUi.kt`
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalUploads.kt`
- `apps/android/app/src/main/res/values-zh/strings.xml`
- `apps/android/app/src/main/res/values/strings.xml`
- `crates/android/Cargo.toml`
- `crates/android/src/terminal.rs`
- `crates/android/src/terminal/navigation.rs`
- `crates/android/src/terminal/upload.rs`
- `crates/cli/Cargo.toml`
- `crates/cli/src/terminal_ui.rs`
- `crates/cli/src/terminal_ui/prefix.rs`
- `crates/cli/src/terminal_ui/session.rs`
- `crates/cli/src/terminal_ui/session_tests.rs`
- `crates/cli/src/terminal_ui/upload.rs`
- `crates/client/src/framing.rs`
- `crates/client/src/handshake.rs`
- `crates/client/src/iroh_controller.rs`
- `crates/client/src/lib.rs`
- `crates/client/src/pair_framing.rs`
- `crates/client/src/protocol.rs`
- `crates/client/src/session.rs`
- `crates/client/src/upload.rs`
- `crates/client/src/view.rs`
- `crates/core/src/domain.rs`
- `crates/core/src/lib.rs`
- `crates/core/src/upload.rs`
- `crates/daemon/Cargo.toml`
- `crates/daemon/src/client/mod.rs`
- `crates/daemon/src/client/transport.rs`
- `crates/daemon/src/connection_broker.rs`
- `crates/daemon/src/lifecycle.rs`
- `crates/daemon/src/operations.rs`
- `crates/daemon/src/remote_tunnel.rs`
- `crates/daemon/src/service.rs`
- `crates/daemon/src/session.rs`
- `crates/daemon/src/session_wire.rs`
- `crates/daemon/src/session_wire/tests/upload_tests.rs`
- `crates/daemon/src/session_wire/upload.rs`
- `crates/platform/src/lib.rs`
- `crates/platform/src/upload.rs`
- `crates/platform/src/user_state.rs`
- `crates/proto/build.rs`
- `crates/proto/src/lib.rs`
- `crates/proto/src/upload.rs`
- `deny.toml`
- `docs/android.md`
- `docs/remote-cli.md`
- `proto/zterm/v2/local.proto`
- `proto/zterm/v2/upload.proto`
- `proto/zterm/v2/wire.proto`

After approval, the Trellis workflow may archive completed tasks and record the
journal in separate bookkeeping commits. Manual platform acceptance gaps stay
explicit; do not silently mark every acceptance checkbox complete.
